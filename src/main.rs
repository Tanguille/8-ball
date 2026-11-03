use std::{io::Read, path::PathBuf, sync::Arc};

use clap::Parser;
use rmcp::ServiceExt;
use serde_json::{Value, json};

use eightball::{
    config::{self, Config},
    question::{Kind, Question},
    service::Service,
};

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
    #[command(flatten)]
    remote: Remote,
}

/// Where the CLI client finds the `8ball serve` REST API.
#[derive(clap::Args)]
struct Remote {
    #[arg(long, env = "EIGHTBALL_URL", default_value = "http://127.0.0.1:8080")]
    url: String,
    #[arg(long, env = "EIGHTBALL_API_KEY", hide_env_values = true)]
    api_key: Option<String>,
}

impl Remote {
    async fn post(&self, path: &str, body: &Value) -> anyhow::Result<Value> {
        self.send(reqwest::Client::new().post(self.at(path)).json(body))
            .await
    }

    async fn get(&self, path: &str) -> anyhow::Result<Value> {
        self.send(reqwest::Client::new().get(self.at(path))).await
    }

    fn at(&self, path: &str) -> String {
        format!("{}{path}", self.url.trim_end_matches('/'))
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> anyhow::Result<Value> {
        response_json(
            eightball::backend::authed(request, self.api_key.as_deref())
                .send()
                .await?,
        )
        .await
    }
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
        #[command(flatten)]
        remote: Remote,
    },
    /// Inspect endpoint detection and health.
    Doctor {
        #[command(flatten)]
        remote: Remote,
    },
    /// Label a saved spec decision.
    Label {
        id: i64,
        value: String,
        #[command(flatten)]
        remote: Remote,
    },
    /// Fit decision bands from labelled states.
    Calibrate {
        spec: String,
        #[arg(long)]
        precision: Option<f64>,
        #[arg(long)]
        dry_run: bool,
        #[command(flatten)]
        remote: Remote,
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
            let server = eightball::mcp::Mcp::new(build_service(config, db).await?)
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
            remote,
        } => {
            let question = make_question(&kind, instructions, options, levels);
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input)?;
            let items: Vec<Value> = serde_json::from_str(&input)?;
            let body = remote
                .post(
                    "/v1/triage",
                    &json!({
                        "items": items,
                        "question": question,
                        "keep": keep,
                        "top_k": top_k,
                        "endpoint": endpoint
                    }),
                )
                .await?;
            println!("{body}");

            Ok(0)
        }
        Command::Doctor { remote } => doctor(&remote).await,
        Command::Label { id, value, remote } => {
            println!(
                "{}",
                remote
                    .post("/v1/label", &json!({"id":id,"value":value}))
                    .await?
            );

            Ok(0)
        }
        Command::Calibrate {
            spec,
            precision,
            dry_run,
            remote,
        } => {
            println!(
                "{}",
                remote
                    .post(
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
            Config::load(&path.unwrap_or_else(config::default_config_path))?;
            println!("ok");

            Ok(0)
        }
    }
}

async fn shutdown() {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = terminate.recv() => {}
    }
}

async fn serve(
    config: Option<PathBuf>,
    listen: String,
    db: Option<PathBuf>,
    api_key: Option<String>,
) -> anyhow::Result<()> {
    let service = build_service(config, db).await?;
    let cancellation_token = tokio_util::sync::CancellationToken::new();
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    let address = listener.local_addr()?;
    if api_key.is_none() && !address.ip().is_loopback() {
        tracing::warn!(
            "no EIGHTBALL_API_KEY set and listening on {address}: anyone on the network can use this server"
        );
    }
    tracing::info!("listening on {address}");
    axum::serve(
        listener,
        eightball::http::router(service, api_key.as_deref(), &cancellation_token),
    )
    .with_graceful_shutdown(async move {
        shutdown().await;
        cancellation_token.cancel();
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
                .map(|text| {
                    text.split_once('=').map_or_else(
                        || (text.clone(), String::new()),
                        |(name, description)| (name.into(), description.into()),
                    )
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

async fn response_json(response: reqwest::Response) -> anyhow::Result<Value> {
    let status = response.status();
    let body: Value = response.json().await?;
    if !status.is_success() {
        anyhow::bail!(
            "HTTP {status}: {}",
            body["error"].as_str().unwrap_or("request failed")
        );
    }

    Ok(body)
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
        remote,
    }: AskArgs,
) -> anyhow::Result<i32> {
    let (questions, noul) = if spec.is_some() {
        (Vec::new(), None)
    } else {
        let (Some(kind), Some(instructions)) = (kind, instructions) else {
            anyhow::bail!("give --spec or <kind> <instructions>");
        };
        let question = make_question(&kind, instructions, options, levels);
        let noul = matches!(question.kind, Kind::Noul { .. });
        (vec![question], Some(noul))
    };
    let state = if let Some(state) = state {
        state
    } else {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text)?;
        text
    };
    let body = remote
        .post(
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
            let report = remote.get("/v1/doctor").await?;
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
    let config = Config::load(&config.unwrap_or_else(config::default_config_path))?;
    let endpoints = config
        .endpoints
        .iter()
        .map(|(name, configuration)| {
            let key = configuration
                .api_key_env
                .as_ref()
                .and_then(|name| std::env::var(name).ok());

            Ok((
                name.clone(),
                eightball::backend::Endpoint::new(name, configuration, key)?,
            ))
        })
        .collect::<Result<indexmap::IndexMap<_, _>, eightball::backend::BackendError>>()?;
    let store =
        eightball::store::Store::open(&db.unwrap_or_else(|| {
            config::xdg_path("XDG_STATE_HOME", ".local/state", "8ball/8ball.db")
        }))
        .await?;
    let pruned = store.prune(config.retention_days).await?;
    tracing::info!("pruned {pruned} old decisions");

    Ok(Arc::new(Service::new(config, endpoints, store)))
}

async fn doctor(remote: &Remote) -> anyhow::Result<i32> {
    let report = remote.get("/v1/doctor").await?;
    println!("{report}");
    let default = report["endpoints"].as_array().and_then(|endpoints| {
        endpoints
            .iter()
            .find(|endpoint| endpoint["name"] == report["default"])
    });

    Ok(
        if default.is_none_or(|endpoint| {
            !endpoint["error"].is_null() || !endpoint["probe"]["error"].is_null()
        }) {
            4
        } else {
            0
        },
    )
}
