use clap::Parser;
use eightball::{
    backend::Endpoint,
    config::{Config, default_config_path},
    http::router,
    question::{Kind, Question},
    service::Service,
    store::Store,
};
use indexmap::IndexMap;
use serde_json::{Value, json};
use std::{io::Read, path::PathBuf, sync::Arc};

#[derive(clap::Parser)]
#[command(
    name = "8ball",
    version,
    about = "Cheap typed judgments from small decision models"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Run the HTTP server (REST + MCP).
    Serve {
        #[arg(long, env = "EIGHTBALL_CONFIG")]
        config: Option<PathBuf>,
        #[arg(long, env = "EIGHTBALL_LISTEN", default_value = "127.0.0.1:8080")]
        listen: String,
        #[arg(long, env = "EIGHTBALL_DB")]
        db: Option<PathBuf>,
    },
    /// Ask an ad-hoc question (state from --state or stdin).
    Ask {
        #[arg(value_parser = ["noul", "choice", "score"])]
        kind: String,
        instructions: String,
        /// choice option, repeatable: name or name=description
        #[arg(long = "option")]
        options: Vec<String>,
        /// score level, repeatable, lowest first
        #[arg(long = "level")]
        levels: Vec<String>,
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        endpoint: Option<String>,
        /// exit 0 yes / 1 no / 3 unsure (noul); 0 decided / 3 unsure (choice, score)
        #[arg(long)]
        exit_code: bool,
        #[arg(long, env = "EIGHTBALL_URL", default_value = "http://127.0.0.1:8080")]
        url: String,
    },
    /// Judge a JSON array from stdin.
    Triage {
        instructions: String,
        #[arg(long = "type", default_value = "noul", value_parser = ["noul", "choice", "score"])]
        kind: String,
        #[arg(long = "option")]
        options: Vec<String>,
        #[arg(long = "level")]
        levels: Vec<String>,
        #[arg(long)]
        keep: Option<String>,
        #[arg(long)]
        top_k: Option<usize>,
        #[arg(long)]
        endpoint: Option<String>,
        #[arg(long, env = "EIGHTBALL_URL", default_value = "http://127.0.0.1:8080")]
        url: String,
    },
    /// Inspect endpoint detection and health.
    Doctor {
        #[arg(long, env = "EIGHTBALL_URL", default_value = "http://127.0.0.1:8080")]
        url: String,
    },
    /// Validate a config file.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}
#[derive(clap::Subcommand)]
enum ConfigAction {
    Check { path: Option<PathBuf> },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let code = match run().await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("8ball: {e:#}");
            4
        }
    };
    std::process::exit(code);
}

async fn run() -> anyhow::Result<i32> {
    match Cli::parse().command {
        Command::Serve { config, listen, db } => {
            serve(config, listen, db).await?;
            Ok(0)
        }
        Command::Ask {
            kind,
            instructions,
            options,
            levels,
            state,
            endpoint,
            exit_code,
            url,
        } => {
            let q = make_question(&kind, instructions, options, levels);
            let noul = matches!(q.kind, Kind::Noul { .. });
            let state = if let Some(state) = state {
                state
            } else {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                s
            };
            let body = post_json(
                &url,
                "/v1/ask",
                &json!({"state":state,"questions":[q],"endpoint":endpoint}),
            )
            .await?;
            println!("{body}");
            let verdict = body["results"][0]["verdict"].as_str().unwrap_or("unsure");
            Ok(if !exit_code {
                0
            } else if verdict == "unsure" {
                3
            } else {
                i32::from(noul && verdict == "no")
            })
        }
        Command::Triage {
            instructions,
            kind,
            options,
            levels,
            keep,
            top_k,
            endpoint,
            url,
        } => {
            let q = make_question(&kind, instructions, options, levels);
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input)?;
            let items: Vec<Value> = serde_json::from_str(&input)?;
            let body = post_json(
                &url,
                "/v1/triage",
                &json!({"items":items,"question":q,"keep":keep,"top_k":top_k,"endpoint":endpoint}),
            )
            .await?;
            println!("{body}");
            Ok(0)
        }
        Command::Doctor { url } => {
            let report: Value = reqwest::get(format!("{}/v1/doctor", url.trim_end_matches('/')))
                .await?
                .error_for_status()?
                .json()
                .await?;
            println!("{report}");
            let default = report["endpoints"]
                .as_array()
                .and_then(|eps| eps.iter().find(|ep| ep["name"] == report["default"]));
            Ok(
                if default
                    .is_none_or(|ep| !ep["error"].is_null() || !ep["probe"]["error"].is_null())
                {
                    4
                } else {
                    0
                },
            )
        }
        Command::Config {
            action: ConfigAction::Check { path },
        } => {
            Config::load(&path.unwrap_or_else(default_config_path))?;
            println!("ok");
            Ok(0)
        }
    }
}

async fn shutdown() {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("SIGTERM handler");
    tokio::select! { _=tokio::signal::ctrl_c()=>{},_=terminate.recv()=>{} }
}

async fn serve(config: Option<PathBuf>, listen: String, db: Option<PathBuf>) -> anyhow::Result<()> {
    let config = Config::load(&config.unwrap_or_else(default_config_path))?;
    let endpoints = config
        .endpoints
        .iter()
        .map(|(name, cfg)| {
            let key = cfg
                .api_key_env
                .as_ref()
                .and_then(|name| std::env::var(name).ok());
            Ok((name.clone(), Endpoint::new(name, cfg, key)?))
        })
        .collect::<Result<IndexMap<_, _>, eightball::backend::BackendError>>()?;
    let store = Store::open(&db.unwrap_or_else(default_db_path))?;
    let n = store.prune(config.retention_days).await?;
    tracing::info!("pruned {n} old decisions");
    let service = Arc::new(Service::new(config, endpoints, store));
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    let addr = listener.local_addr()?;
    tracing::info!("listening on {addr}");
    axum::serve(listener, router(service))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

fn make_question(
    kind: &str,
    instructions: String,
    options: Vec<String>,
    levels: Vec<String>,
) -> Question {
    let kind = match kind {
        "noul" => Kind::Noul { criteria: None },
        "choice" => Kind::Choice {
            criteria: options
                .into_iter()
                .map(|s| {
                    s.split_once('=')
                        .map_or_else(|| (s.clone(), String::new()), |(a, b)| (a.into(), b.into()))
                })
                .collect(),
        },
        "score" => Kind::Score { criteria: levels },
        _ => unreachable!("clap validates kind"),
    };
    Question {
        id: "q".into(),
        instructions,
        kind,
    }
}

async fn post_json(url: &str, path: &str, body: &Value) -> anyhow::Result<Value> {
    let response = reqwest::Client::new()
        .post(format!("{}{path}", url.trim_end_matches('/')))
        .json(body)
        .send()
        .await?;
    let status = response.status();
    let body: Value = response.json().await?;
    if !status.is_success() {
        anyhow::bail!("{}", body["error"].as_str().unwrap_or("request failed"));
    }
    Ok(body)
}

fn default_db_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map_or_else(
            || PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state"),
            PathBuf::from,
        )
        .join("8ball/8ball.db")
}
