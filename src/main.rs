use clap::Parser;
use eightball::{
    backend::Endpoint,
    config::{Config, default_config_path},
    http::router,
    mcp::Mcp,
    question::{Kind, Question},
    service::Service,
    store::Store,
};
use indexmap::IndexMap;
use rmcp::ServiceExt;
use serde_json::{Value, json};
use std::{io::Read, path::PathBuf, sync::Arc};
use tokio_util::sync::CancellationToken;

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

#[derive(clap::Args)]
struct AskArgs {
    #[arg(value_parser = ["noul", "choice", "score"])]
    kind: Option<String>,
    instructions: Option<String>,
    #[arg(long)]
    spec: Option<String>,
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
        #[arg(long, env = "EIGHTBALL_API_KEY", hide_env_values = true)]
        api_key: Option<String>,
    },
    /// Run MCP over stdin and stdout.
    Mcp {
        #[arg(long, env = "EIGHTBALL_CONFIG")]
        config: Option<PathBuf>,
        #[arg(long, env = "EIGHTBALL_DB")]
        db: Option<PathBuf>,
    },
    /// Ask an ad-hoc question (state from --state or stdin).
    Ask(AskArgs),
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
    /// Label a saved spec decision.
    Label {
        id: i64,
        value: String,
        #[arg(long, env = "EIGHTBALL_URL", default_value = "http://127.0.0.1:8080")]
        url: String,
    },
    /// Fit decision bands from labelled states.
    Calibrate {
        spec: String,
        #[arg(long)]
        precision: Option<f64>,
        #[arg(long)]
        dry_run: bool,
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
        Command::Serve {
            config,
            listen,
            db,
            api_key,
        } => {
            serve(config, listen, db, api_key).await?;
            Ok(0)
        }
        Command::Mcp { config, db } => {
            let server = Mcp::new(build_service(config, db).await?)
                .serve(rmcp::transport::stdio())
                .await?;
            server.waiting().await?;
            Ok(0)
        }
        Command::Ask(args) => ask(args).await,
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
            let report = doctor_report(&url).await?;
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
        Command::Label { id, value, url } => {
            println!(
                "{}",
                post_json(&url, "/v1/label", &json!({"id":id,"value":value})).await?
            );
            Ok(0)
        }
        Command::Calibrate {
            spec,
            precision,
            dry_run,
            url,
        } => {
            println!(
                "{}",
                post_json(
                    &url,
                    "/v1/calibrate",
                    &json!({"spec":spec,"precision":precision,"dry_run":dry_run})
                )
                .await?
            );
            Ok(0)
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

async fn serve(
    config: Option<PathBuf>,
    listen: String,
    db: Option<PathBuf>,
    api_key: Option<String>,
) -> anyhow::Result<()> {
    let service = build_service(config, db).await?;
    let ct = CancellationToken::new();
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    let addr = listener.local_addr()?;
    tracing::info!("listening on {addr}");
    axum::serve(listener, router(service, api_key.as_deref(), &ct))
        .with_graceful_shutdown(async move {
            shutdown().await;
            ct.cancel();
        })
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

async fn doctor_report(url: &str) -> anyhow::Result<Value> {
    Ok(
        reqwest::get(format!("{}/v1/doctor", url.trim_end_matches('/')))
            .await?
            .error_for_status()?
            .json()
            .await?,
    )
}

async fn ask(
    AskArgs {
        kind,
        instructions,
        spec,
        options,
        levels,
        state,
        endpoint,
        exit_code,
        url,
    }: AskArgs,
) -> anyhow::Result<i32> {
    let (questions, noul) = if spec.is_some() {
        (Vec::new(), None)
    } else {
        let (Some(kind), Some(instructions)) = (kind, instructions) else {
            anyhow::bail!("give --spec or <kind> <instructions>");
        };
        let q = make_question(&kind, instructions, options, levels);
        let noul = matches!(q.kind, Kind::Noul { .. });
        (vec![q], Some(noul))
    };
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
        &json!({"state":state,"questions":questions,"spec":spec,"endpoint":endpoint}),
    )
    .await?;
    println!("{body}");
    let verdict = body["results"][0]["verdict"].as_str().unwrap_or("unsure");
    Ok(if !exit_code {
        0
    } else if verdict == "unsure" {
        3
    } else {
        let noul = if verdict != "no" {
            false
        } else if let Some(noul) = noul {
            noul
        } else {
            let report = doctor_report(&url).await?;
            report["specs"]
                .as_array()
                .and_then(|specs| {
                    specs
                        .iter()
                        .find(|item| item["name"] == spec.as_deref().unwrap_or_default())
                })
                .is_some_and(|item| item["type"] == "noul")
        };
        i32::from(noul && verdict == "no")
    })
}

async fn build_service(
    config: Option<PathBuf>,
    db: Option<PathBuf>,
) -> anyhow::Result<Arc<Service>> {
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
    Ok(Arc::new(Service::new(config, endpoints, store)))
}
