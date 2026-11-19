use std::{
    fs,
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    sync::Arc,
    time::Instant,
};

use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};
use reqwest::Client;
use rmcp::ServiceExt;
use serde_json::{Value, json};
use tracing::info;

use domain::{
    config::{self, Config},
    question::{Kind, Question},
};
use service::{MAX_TRIAGE_ITEMS, Service, TriageResponse};

mod evaluate;

#[derive(Parser)]
#[command(
    name = "8ball",
    version,
    about = "Cheap typed judgments from small decision models"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct AskArgs {
    #[arg(value_parser = ["noul", "choice", "score"])]
    kind: Option<String>,
    instructions: Option<String>,
    #[arg(long, conflicts_with_all = ["kind", "instructions", "options", "levels"])]
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
    /// Turn off remembered labels, exploration and reports, for scripts that never label.
    #[arg(long)]
    no_learn: bool,
    #[command(flatten)]
    remote: Remote,
}

#[derive(Args)]
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
    /// Turn off remembered labels, exploration and reports, for scripts that never label.
    #[arg(long)]
    no_learn: bool,
    #[command(flatten)]
    remote: Remote,
}

#[derive(Args)]
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

#[derive(Args)]
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
#[derive(Args)]
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
    fn client(&self) -> Result<Client> {
        let mut builder = Client::builder().redirect(reqwest::redirect::Policy::none());
        if let Some(timeout_ms) = self.timeout_ms {
            builder = builder.timeout(std::time::Duration::from_millis(timeout_ms));
        }

        Ok(builder.build()?)
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        self.send(self.client()?.post(self.at(path)).json(body))
            .await
    }

    async fn print(&self, path: &str, body: &Value) -> Result<i32> {
        println!("{}", self.post(path, body).await?);

        Ok(0)
    }

    fn at(&self, path: &str) -> String {
        format!("{}{path}", self.url.trim_end_matches('/'))
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Value> {
        let response = backend::authed(request, self.api_key.as_deref())
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body: Value = response.json().await.unwrap_or_default();
            bail!(
                "HTTP {status}: {}",
                body["error"].as_str().unwrap_or("request failed")
            );
        }

        Ok(response.json().await?)
    }
}

/// Where `serve` and `mcp` read their configuration and keep their database.
#[derive(Args)]
struct ConfigArgs {
    #[arg(long, env = "EIGHTBALL_CONFIG")]
    config: Option<PathBuf>,
    /// Configuration TOML text, instead of a file.
    #[arg(
        long,
        env = "EIGHTBALL_CONFIG_TOML",
        conflicts_with = "config",
        hide_env_values = true
    )]
    config_toml: Option<String>,
    #[arg(long, env = "EIGHTBALL_DB")]
    db: Option<PathBuf>,
}

impl ConfigArgs {
    async fn service(self) -> Result<Arc<Service>> {
        let config = match self.config_toml {
            Some(text) => Config::parse(&text, "EIGHTBALL_CONFIG_TOML")?,
            None => Config::load(&self.config.unwrap_or_else(config::default_config_path))?,
        };
        let store = store::Store::open(&self.db.unwrap_or_else(|| {
            config::xdg_path("XDG_STATE_HOME", ".local/state", "8ball/8ball.db")
        }))
        .await?;
        let pruned = store.prune(config.retention_days).await?;
        info!("pruned {pruned} old decisions");

        Ok(Arc::new(Service::new(config, store)))
    }
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP server (REST + MCP).
    Serve {
        #[command(flatten)]
        config: ConfigArgs,
        #[arg(long, env = "EIGHTBALL_LISTEN", default_value = "127.0.0.1:8080")]
        listen: String,
        #[arg(
            long,
            env = "EIGHTBALL_API_KEY",
            hide_env_values = true,
            value_parser = clap::builder::NonEmptyStringValueParser::new()
        )]
        api_key: Option<String>,
    },
    /// Run MCP over stdin and stdout.
    Mcp(ConfigArgs),
    /// Ask an ad-hoc question or a --spec (state from --state or stdin).
    Ask(AskArgs),
    /// Judge a JSON array from stdin.
    Triage(TriageArgs),
    /// Replay labelled JSONL cases from stdin without changing calibration.
    Evaluate(EvaluateArgs),
    /// Record triage decisions and print every original candidate unfiltered.
    Observe(ObserveArgs),
    /// Inspect endpoint detection and health.
    Doctor {
        #[command(flatten)]
        remote: Remote,
    },
    /// Label a saved decision (a current spec, or an ad-hoc question when `learning.ad_hoc` is on).
    Label {
        id: i64,
        value: String,
        #[command(flatten)]
        remote: Remote,
    },
    /// List stored questions, most decisions first, with decision, unsure and label counts.
    Questions {
        #[arg(long)]
        limit: Option<usize>,
        #[command(flatten)]
        remote: Remote,
    },
    /// List the stored decisions of a spec (current version) or an ad-hoc question, to review and label.
    Decisions {
        #[arg(long, required_unless_present = "question")]
        spec: Option<String>,
        /// Question hash from `8ball questions`; with --spec, an older spec version.
        #[arg(long)]
        question: Option<String>,
        #[arg(long)]
        unlabelled: bool,
        #[arg(long)]
        limit: Option<usize>,
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

#[derive(Subcommand)]
enum ConfigAction {
    Check { path: Option<PathBuf> },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                // rmcp logs every MCP session at info; a polling client buried everything else.
                .unwrap_or_else(|_| "info,rmcp=warn".into()),
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

async fn run() -> Result<i32> {
    match Cli::parse().command {
        Command::Serve {
            config,
            listen,
            api_key,
        } => {
            serve(config, listen, api_key).await?;

            Ok(0)
        }
        Command::Mcp(config) => {
            let server = service::mcp::Mcp::new(config.service().await?)
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
            remote
                .print("/v1/label", &json!({"id":id,"value":value}))
                .await
        }
        Command::Questions { limit, remote } => {
            remote.print("/v1/questions", &json!({"limit":limit})).await
        }
        Command::Decisions {
            spec,
            question,
            unlabelled,
            limit,
            remote,
        } => {
            remote
                .print(
                    "/v1/decisions",
                    &json!({"spec":spec,"question":question,"unlabelled":unlabelled,"limit":limit}),
                )
                .await
        }
        Command::Calibrate {
            spec,
            precision,
            dry_run,
            remote,
        } => {
            remote
                .print(
                    "/v1/calibrate",
                    &json!({"spec":spec,"precision":precision,"dry_run":dry_run}),
                )
                .await
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

async fn serve(config: ConfigArgs, listen: String, api_key: Option<String>) -> Result<()> {
    let service = config.service().await?;
    let cancellation_token = tokio_util::sync::CancellationToken::new();
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    let address = listener.local_addr()?;
    if api_key.is_none() && !address.ip().is_loopback() {
        tracing::warn!(
            "no EIGHTBALL_API_KEY set and listening on {address}: anyone on the network can use this server"
        );
    }
    info!("listening on {address}");
    axum::serve(
        listener,
        service::http::router(service, api_key.as_deref(), &cancellation_token),
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
                    let (name, description) = text.split_once('=').unwrap_or((&text, ""));
                    (name.into(), description.into())
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
        no_learn,
        remote,
    }: AskArgs,
) -> Result<i32> {
    let questions = if spec.is_some() {
        Vec::new()
    } else {
        let (Some(kind), Some(instructions)) = (kind, instructions) else {
            bail!("give --spec or <kind> <instructions>");
        };

        vec![make_question(&kind, instructions, options, levels)]
    };
    let state = state.map_or_else(|| io::read_to_string(io::stdin()), Ok)?;
    let body = remote
        .post(
            "/v1/ask",
            &json!({"state":state,"questions":questions,"spec":spec,"endpoint":endpoint,
            "learn":!no_learn}),
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

async fn doctor(remote: &Remote) -> Result<i32> {
    let report = remote
        .send(remote.client()?.get(remote.at("/v1/doctor")))
        .await?;
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

async fn triage(args: TriageArgs) -> Result<i32> {
    let question = args
        .instructions
        .map(|instructions| make_question(&args.kind, instructions, args.options, args.levels));
    let context = args.context_file.map(fs::read_to_string).transpose()?;
    let items: Vec<Value> = serde_json::from_str(&io::read_to_string(io::stdin())?)?;
    let mut body = json!({
        "context":context, "spec":args.spec, "question":question, "keep":args.keep,
        "top_k":args.top_k, "endpoint":args.endpoint, "learn":!args.no_learn
    });
    // ponytail: chunks go one at a time, and top_k and duplicate sharing work per chunk; send them in parallel or
    // split on the server if large arrays need that.
    // An empty array still makes one request, so the server checks the question.
    let chunks = items
        .chunks(MAX_TRIAGE_ITEMS)
        .chain(items.is_empty().then_some(&[][..]));
    let client = args.remote.client()?;
    let mut merged = TriageResponse::default();
    let mut warnings = Vec::new();
    for (start, chunk) in (0..).step_by(MAX_TRIAGE_ITEMS).zip(chunks) {
        body["items"] = json!(chunk);
        let request = client.post(args.remote.at("/v1/triage")).json(&body);
        let response = serde_json::from_value(args.remote.send(request).await?)?;
        merge(&mut merged, &mut warnings, response, start);
    }
    merged.warning = (!warnings.is_empty()).then(|| warnings.join("; "));
    println!("{}", json!(merged));

    Ok(0)
}

/// Appends the response for the chunk whose first index is `start`. It takes every field by name, so a new field
/// fails to compile here instead of keeping only the first chunk's value.
fn merge(
    merged: &mut TriageResponse,
    warnings: &mut Vec<String>,
    response: TriageResponse,
    start: usize,
) {
    let TriageResponse {
        kept,
        unsure,
        dropped,
        rejected,
        prefiltered,
        warning,
    } = response;
    for (all, items) in [
        (&mut merged.kept, kept),
        (&mut merged.unsure, unsure),
        (&mut merged.rejected, rejected),
    ] {
        all.extend(items.into_iter().map(|mut item| {
            item.index += start;
            item
        }));
    }
    merged
        .prefiltered
        .extend(prefiltered.into_iter().map(|index| index + start));
    merged.dropped += dropped;
    warnings.extend(warning);
}

async fn evaluate(mut args: EvaluateArgs) -> Result<i32> {
    args.remote.timeout_ms.get_or_insert(25_000);
    let cases = evaluate::parse(&io::read_to_string(io::stdin())?)?;
    let client = args.remote.client()?;
    let mut observations = Vec::with_capacity(cases.len());
    for case in cases {
        let start = Instant::now();
        let response = args
            .remote
            .send(client.post(args.remote.at("/v1/triage")).json(
                &json!({"spec":args.spec,"endpoint":args.endpoint,"keep":args.keep,
                "items":[case.item],"context":case.context,"learn":false}),
            ))
            .await
            .and_then(|body| Ok(serde_json::from_value::<TriageResponse>(body)?))
            .map_err(|e| format!("{e:#}"));
        observations.push(evaluate::Observation::new(case, start.elapsed(), response));
    }
    let report = evaluate::Report::from_observations(&observations);
    let code = if report.errors == 0 { 0 } else { 4 };
    println!(
        "{}",
        json!({"spec":args.spec,"endpoint":args.endpoint,"keep":args.keep,
        "report":report,"observations":observations})
    );

    Ok(code)
}

async fn observe(mut args: ObserveArgs) -> Result<i32> {
    args.remote.timeout_ms.get_or_insert(25_000);
    let mut context = json!({"mode":"observe"});
    let snapshot = (|| -> Result<Value> {
        let snapshot: Value = serde_json::from_str(&fs::read_to_string(&args.input)?)?;
        anyhow::ensure!(
            snapshot["context"].is_string()
                && snapshot["items"]
                    .as_array()
                    .is_some_and(|items| items.len() <= MAX_TRIAGE_ITEMS),
            "snapshot requires context text and an items array of at most {MAX_TRIAGE_ITEMS} candidates"
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
    let start = Instant::now();
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
    let error = response.as_ref().err().map(|e| format!("{e:#}"));
    let result = response.ok();
    let record = json!({"mode":"observe", "spec":args.spec, "endpoint":args.endpoint,
        "snapshot":snapshot, "result":result, "error":error,
        "elapsed_ms":start.elapsed().as_secs_f64() * 1000.0,
        "observed_at_unix_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().map(|elapsed| elapsed.as_millis())});
    let saved = (|| -> io::Result<()> {
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
    // Keep verdicts out of the printed evidence so the observation does not bias its consumer.
    context["snapshot"] = snapshot;
    println!("{}", json!({"wakeAgent":true,"context":context}));

    Ok(0)
}
