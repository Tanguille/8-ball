use std::{fs, io::Write, os::unix::fs::OpenOptionsExt, path::PathBuf, sync::Arc};

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

#[derive(clap::Args)]
struct TriageArgs {
    #[arg(required_unless_present = "spec", conflicts_with = "spec")]
    instructions: Option<String>,
    #[arg(long, conflicts_with_all = ["kind", "options", "levels"])]
    spec: Option<String>,
    /// UTF-8 file containing shared evidence for every item.
    #[arg(long)]
    context_file: Option<PathBuf>,
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
}

#[derive(clap::Args)]
struct EvaluateArgs {
    #[arg(long)]
    spec: String,
    #[arg(long)]
    endpoint: Option<String>,
    #[arg(long, default_value = "yes")]
    keep: String,
    #[command(flatten)]
    remote: Remote,
}

#[derive(clap::Args)]
struct ObserveArgs {
    #[arg(long)]
    spec: String,
    #[arg(long)]
    endpoint: Option<String>,
    #[arg(long)]
    input: PathBuf,
    /// New private JSONL file; existing files are never overwritten.
    #[arg(long)]
    observations: PathBuf,
    #[command(flatten)]
    remote: Remote,
}

/// Where the CLI client finds the `8ball serve` REST API.
#[derive(clap::Args)]
struct Remote {
    /// Total request deadline; defaults to 25000 for observe/evaluate, otherwise unset.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    timeout_ms: Option<u64>,
    #[arg(long, env = "EIGHTBALL_URL", default_value = "http://127.0.0.1:8080")]
    url: String,
    #[arg(long, env = "EIGHTBALL_API_KEY", hide_env_values = true)]
    api_key: Option<String>,
}

impl Remote {
    fn client(&self) -> anyhow::Result<reqwest::Client> {
        let mut builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
        if let Some(timeout_ms) = self.timeout_ms {
            builder = builder.timeout(std::time::Duration::from_millis(timeout_ms));
        }

        Ok(builder.build()?)
    }

    async fn post(&self, path: &str, body: &Value) -> anyhow::Result<Value> {
        self.send(self.client()?.post(self.at(path)).json(body))
            .await
    }

    async fn get(&self, path: &str) -> anyhow::Result<Value> {
        self.send(self.client()?.get(self.at(path))).await
    }

    fn at(&self, path: &str) -> String {
        format!("{}{path}", self.url.trim_end_matches('/'))
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> anyhow::Result<Value> {
        let response = eightball::backend::authed(request, self.api_key.as_deref())
            .send()
            .await?;
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
    Triage(TriageArgs),
    /// Replay labelled JSONL cases from stdin without changing calibration.
    Evaluate(EvaluateArgs),
    /// Record triage decisions and always wake Hermes with every original candidate.
    Observe(ObserveArgs),
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
        Command::Triage(args) => triage(args).await,
        Command::Evaluate(args) => evaluate(args).await,
        Command::Observe(args) => observe(args).await,
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
    let questions = if spec.is_some() {
        Vec::new()
    } else {
        let (Some(kind), Some(instructions)) = (kind, instructions) else {
            anyhow::bail!("give --spec or <kind> <instructions>");
        };
        vec![make_question(&kind, instructions, options, levels)]
    };
    let state = if let Some(state) = state {
        state
    } else {
        std::io::read_to_string(std::io::stdin())?
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
        i32::from(verdict == "no" && body["results"][0]["type"] == "noul")
    })
}

async fn build_service(
    config: Option<PathBuf>,
    db: Option<PathBuf>,
) -> anyhow::Result<Arc<Service>> {
    let config = Config::load(&config.unwrap_or_else(config::default_config_path))?;
    let store =
        eightball::store::Store::open(&db.unwrap_or_else(|| {
            config::xdg_path("XDG_STATE_HOME", ".local/state", "8ball/8ball.db")
        }))
        .await?;
    let pruned = store.prune(config.retention_days).await?;
    tracing::info!("pruned {pruned} old decisions");

    Ok(Arc::new(Service::new(config, store)))
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

async fn triage(args: TriageArgs) -> anyhow::Result<i32> {
    let question = args
        .instructions
        .map(|instructions| make_question(&args.kind, instructions, args.options, args.levels));
    let context = args.context_file.map(fs::read_to_string).transpose()?;
    let items: Vec<Value> = serde_json::from_str(&std::io::read_to_string(std::io::stdin())?)?;
    let body = args
        .remote
        .post(
            "/v1/triage",
            &json!({
                "items":items, "context":context, "spec":args.spec,
                "question":question, "keep":args.keep, "top_k":args.top_k, "endpoint":args.endpoint
            }),
        )
        .await?;
    println!("{body}");

    Ok(0)
}

async fn evaluate(mut args: EvaluateArgs) -> anyhow::Result<i32> {
    args.remote.timeout_ms.get_or_insert(25_000);
    let cases = eightball::evaluate::parse(&std::io::read_to_string(std::io::stdin())?)?;
    let client = args.remote.client()?;
    let mut observations = Vec::with_capacity(cases.len());
    for case in cases {
        let start = std::time::Instant::now();
        let response = async {
            let body = args
                .remote
                .send(client.post(args.remote.at("/v1/triage")).json(
                    &json!({"spec":args.spec,"endpoint":args.endpoint,"keep":args.keep,
                    "items":[case.item],"context":case.context}),
                ))
                .await?;

            Ok::<_, anyhow::Error>(
                serde_json::from_value::<eightball::service::TriageResponse>(body)?,
            )
        }
        .await
        .map_err(|e| format!("{e:#}"));
        observations.push(eightball::evaluate::Observation::new(
            case,
            start.elapsed(),
            response,
        ));
    }
    let report = eightball::evaluate::Report::from_observations(&observations);
    let code = if report.errors == 0 { 0 } else { 4 };
    println!(
        "{}",
        json!({"spec":args.spec,"endpoint":args.endpoint,"keep":args.keep,
        "report":report,"observations":observations})
    );

    Ok(code)
}

async fn observe(mut args: ObserveArgs) -> anyhow::Result<i32> {
    args.remote.timeout_ms.get_or_insert(25_000);
    let mut context = json!({"mode":"observe"});
    let snapshot = (|| -> anyhow::Result<Value> {
        let snapshot: Value = serde_json::from_str(&fs::read_to_string(&args.input)?)?;
        anyhow::ensure!(
            snapshot["context"].is_string()
                && snapshot["items"]
                    .as_array()
                    .is_some_and(|items| items.len() <= 64),
            "snapshot requires context text and an items array of at most 64 candidates"
        );

        Ok(snapshot)
    })();
    let snapshot = match snapshot {
        Ok(snapshot) => snapshot,
        Err(e) => {
            context["input_error"] = json!(format!("{e:#}"));
            println!("{}", json!({"wakeAgent":true,"context":context}));

            return Ok(0);
        }
    };
    let start = std::time::Instant::now();
    let response = args
        .remote
        .post(
            "/v1/triage",
            &json!({
                "items":snapshot["items"], "context":snapshot["context"],
                "spec":args.spec, "endpoint":args.endpoint
            }),
        )
        .await;
    let (result, error) = match response {
        Ok(result) => (Some(result), None),
        Err(e) => (None, Some(format!("{e:#}"))),
    };
    let record = json!({"mode":"observe", "spec":args.spec, "endpoint":args.endpoint,
        "snapshot":snapshot, "result":result, "error":error,
        "elapsed_ms":start.elapsed().as_secs_f64() * 1000.0,
        "observed_at_unix_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().map(|elapsed| elapsed.as_millis())});
    let saved = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&args.observations)?;

        writeln!(file, "{record}")
    })();
    match saved {
        Ok(()) => context["observations"] = json!(args.observations.to_string_lossy()),
        Err(e) => context["observation_error"] = json!(e.to_string()),
    }
    if let Some(error) = error {
        context["decision_error"] = json!(error);
    }
    // Keep verdicts out of Hermes' evidence so the observation does not bias its work.
    context["snapshot"] = snapshot;
    println!("{}", json!({"wakeAgent":true,"context":context}));

    Ok(0)
}
