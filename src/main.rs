use clap::Parser;
use eightball::{
    backend::Endpoint,
    config::{Config, default_config_path},
    http::router,
    question::{Kind, Question},
    service::Service,
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
        Command::Serve { config, listen } => {
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
            let service = Arc::new(Service::new(config, endpoints));
            let listener = tokio::net::TcpListener::bind(&listen).await?;
            let addr = listener.local_addr()?;
            tracing::info!("listening on {addr}");
            axum::serve(listener, router(service))
                .with_graceful_shutdown(shutdown())
                .await?;
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
            let kind = match kind.as_str() {
                "noul" => Kind::Noul { criteria: None },
                "choice" => Kind::Choice {
                    criteria: options
                        .into_iter()
                        .map(|s| {
                            s.split_once('=').map_or_else(
                                || (s.clone(), String::new()),
                                |(a, b)| (a.into(), b.into()),
                            )
                        })
                        .collect(),
                },
                "score" => Kind::Score { criteria: levels },
                _ => unreachable!("clap validates kind"),
            };
            let noul = matches!(kind, Kind::Noul { .. });
            let q = Question {
                id: "q".into(),
                instructions,
                kind,
            };
            let state = if let Some(state) = state {
                state
            } else {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                s
            };
            let response = reqwest::Client::new()
                .post(format!("{}/v1/ask", url.trim_end_matches('/')))
                .json(&json!({"state":state,"questions":[q],"endpoint":endpoint}))
                .send()
                .await?;
            let status = response.status();
            let body: Value = response.json().await?;
            if !status.is_success() {
                anyhow::bail!("{}", body["error"].as_str().unwrap_or("request failed"));
            }
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
