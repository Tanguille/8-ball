//! Benchmark every model of the local stack on fixed public cases. See README "Benchmark".
//!
//! Usage: `dev/models.sh up && dev/models.sh bench && cargo build --release -p cli -p bench`,
//! then `cargo run --release -p bench -- [--per-side 100]`.

use std::{
    collections::{BTreeMap, HashMap},
    fmt::Write as _,
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context as _, Result, bail, ensure};
use clap::Parser;
use parquet::{
    file::reader::{FileReader as _, SerializedFileReader},
    record::Field,
};
use serde_json::{Value, json};

const URL: &str = "http://127.0.0.1:8098";

/// A yes/no gate over a public dataset. `yes` holds the labels where the answer is yes.
struct Task {
    spec: &'static str,
    repo: &'static str,
    sha: &'static str,
    path: &'static str,
    field: &'static str,
    yes: &'static [i64],
}

// Pinned revisions, so that every run judges the same cases.
const TASKS: [Task; 3] = [
    Task {
        spec: "spam",
        repo: "ucirvine/sms_spam",
        sha: "cae486f927c250fe1d4a5b55f11357964ed1646c",
        path: "plain_text/train-00000-of-00001.parquet",
        field: "sms",
        yes: &[1],
    },
    Task {
        spec: "positive",
        repo: "stanfordnlp/sst2",
        sha: "8d51e7e4887a4caaa95b3fbebbf53c0490b58bbb",
        path: "data/validation-00000-of-00001.parquet",
        field: "sentence",
        yes: &[1],
    },
    Task {
        spec: "lost-card",
        repo: "PolyAI/banking77",
        sha: "1846984189f2bc0ac51c6fc3a600fa9e1714e0c6",
        path: "data/test-00000-of-00001.parquet",
        field: "text",
        yes: &[22, 41], // compromised_card, lost_or_stolen_card
    },
];

/// The table rows: the endpoints of dev/config.toml and dev/bench.toml, then the cascade,
/// which reuses the observations of `D3` and `BIG` instead of calling a model.
const METHODS: [(&str, &str); 5] = [
    ("d3", "vllm-d3-nano (systemone)"),
    ("rerank", "bge-reranker-v2-m3 (rerank)"),
    ("chat", "Qwen3.5 0.8B (logprob)"),
    ("big", "Qwen3.8 27B (logprob)"),
    ("cascade", "d3, then 27B for unsure"),
];
const ENDPOINTS: usize = 4;
const D3: usize = 0;
const BIG: usize = 3;

#[derive(Parser)]
struct Args {
    /// Cases per expected answer, per task.
    #[arg(long, default_value_t = 100)]
    per_side: usize,
    /// Directory for the dataset files and observations.json.
    #[arg(long, default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../target/bench"))]
    out: PathBuf,
}

/// Observations of one method, per task in `TASKS` order.
type Runs = Vec<Vec<Value>>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();
    fs::create_dir_all(&args.out)?;
    let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("../dev");
    let config = fs::read_to_string(dev.join("config.toml"))?
        + "\n"
        + &fs::read_to_string(dev.join("bench.toml"))?;

    // A new database for each run, so no answer comes from the cache.
    let tmp = tempfile::tempdir()?;
    let db = tmp.path().join("bench.db");
    let mut server = eightball()?
        .args([
            "serve",
            "--config-toml",
            &config,
            "--listen",
            URL.trim_start_matches("http://"),
        ])
        .arg("--db")
        .arg(&db)
        .stderr(Stdio::null())
        .spawn()
        .context("start 8ball serve")?;
    let result = run(&args).await;
    server.kill().ok();
    server.wait().ok();
    let (runs, doctor) = result?;

    let saved: BTreeMap<_, BTreeMap<_, _>> = METHODS
        .iter()
        .zip(&runs)
        .map(|((key, _), by_task)| (key, TASKS.iter().map(|t| t.spec).zip(by_task).collect()))
        .collect();
    fs::write(
        args.out.join("observations.json"),
        serde_json::to_string(&saved)?,
    )?;
    report(&runs, &doctor);

    Ok(())
}

/// The 8ball built next to this binary, so the benchmark measures this checkout, never an older install.
fn eightball() -> Result<Command> {
    let path = std::env::current_exe()?.with_file_name("8ball");
    ensure!(
        path.exists(),
        "{} not found; run `cargo build --release -p cli -p bench`",
        path.display()
    );
    let mut command = Command::new(path);
    command.env("EIGHTBALL_URL", URL);

    Ok(command)
}

async fn wait_for_server() -> Result<()> {
    for _ in 0..50 {
        if reqwest::get(format!("{URL}/healthz")).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    bail!("8ball serve did not start on {URL}")
}

async fn run(args: &Args) -> Result<(Vec<Runs>, Value)> {
    wait_for_server().await?;
    let doctor = serde_json::from_slice(&eightball()?.arg("doctor").output()?.stdout)?;

    let mut runs: Vec<Runs> = vec![Vec::new(); ENDPOINTS];
    for task in &TASKS {
        let mut lines = String::new();
        for case in cases(task, args.per_side, &args.out).await? {
            writeln!(lines, "{case}")?;
        }
        for ((endpoint, _), by_task) in METHODS[..ENDPOINTS].iter().zip(&mut runs) {
            eprintln!("{} / {endpoint} ...", task.spec);
            by_task.push(evaluate(task.spec, endpoint, &lines)?);
        }
    }
    let cascade = runs[D3]
        .iter()
        .zip(&runs[BIG])
        .map(|(d3, big)| cascade(d3, big))
        .collect();
    runs.push(cascade);

    Ok((runs, doctor))
}

async fn cases(task: &Task, per_side: usize, cache: &Path) -> Result<Vec<Value>> {
    let file = cache.join(format!(
        "{}-{}.parquet",
        task.repo.replace('/', "__"),
        &task.sha[..12]
    ));
    if !file.exists() {
        let url = format!(
            "https://huggingface.co/datasets/{}/resolve/{}/{}",
            task.repo, task.sha, task.path
        );
        fs::write(
            &file,
            reqwest::get(url).await?.error_for_status()?.bytes().await?,
        )?;
    }
    // SMS spam repeats some messages, so keep one case per trimmed text. The last row wins.
    let mut rows: HashMap<String, (String, bool)> = HashMap::new();
    for row in SerializedFileReader::new(fs::File::open(&file)?)?.get_row_iter(None)? {
        let (mut text, mut label) = (None, None);
        for (name, value) in row?.get_column_iter() {
            match value {
                Field::Str(s) if name == task.field => text = Some(s.clone()),
                Field::Long(l) if name == "label" => label = Some(*l),
                _ => {}
            }
        }
        let (Some(text), Some(label)) = (text, label) else {
            bail!("{}: row without {} or label", task.repo, task.field);
        };
        rows.insert(text.trim().to_owned(), (text, task.yes.contains(&label)));
    }
    // The id hashes the raw text; sorting by it gives a fixed sample that no one picked by hand.
    let mut all: Vec<_> = rows
        .into_iter()
        .map(|(item, (raw, keep))| (domain::verdict::short_hash(&raw), item, keep))
        .collect();
    all.sort_unstable();
    let side = |keep: bool| all.iter().filter(move |c| c.2 == keep).take(per_side);

    Ok(side(true)
        .chain(side(false))
        .map(|(id, item, keep)| json!({"id": id, "item": item, "expected_keep": keep}))
        .collect())
}

fn evaluate(spec: &str, endpoint: &str, lines: &str) -> Result<Vec<Value>> {
    let mut child = eightball()?
        .args([
            "evaluate",
            "--spec",
            spec,
            "--endpoint",
            endpoint,
            "--timeout-ms",
            "120000",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // evaluate reads all of stdin before it prints, so writing first cannot deadlock.
    child
        .stdin
        .take()
        .context("stdin")?
        .write_all(lines.as_bytes())?;
    let output = child.wait_with_output()?;
    ensure!(
        matches!(output.status.code(), Some(0 | 4)),
        "8ball evaluate {spec} {endpoint} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut output: Value = serde_json::from_slice(&output.stdout)?;
    // The answer cache is keyed by model, not endpoint: a cached answer would fake the time.
    ensure!(
        output["report"]["cached"] == 0,
        "{spec} {endpoint}: cached answers; give each endpoint its own model"
    );

    match output["observations"].take() {
        Value::Array(observations) => Ok(observations),
        _ => bail!("{spec} {endpoint}: no observations"),
    }
}

/// d3 decides; its unsure cases go to the 27B. The time is the sum of both calls.
/// evaluate keeps the input order, so both lists line up.
fn cascade(first: &[Value], second: &[Value]) -> Vec<Value> {
    first
        .iter()
        .zip(second)
        .map(|(a, b)| {
            if verdict(a) != "unsure" {
                return a.clone();
            }
            let total = elapsed(a) + elapsed(b);
            let mut b = b.clone();
            b["elapsed_ms"] = json!(total);
            b
        })
        .collect()
}

fn verdict(o: &Value) -> &str {
    o["decision"]["verdict"]
        .as_str()
        .filter(|_| o["error"].is_null())
        .unwrap_or("error")
}

fn elapsed(o: &Value) -> f64 {
    o["elapsed_ms"].as_f64().unwrap_or_default()
}

#[derive(Default)]
struct Score {
    total: u32,
    decided: u32,
    correct: u32,
    errors: u32,
    elapsed_ms: f64,
}

impl Score {
    fn of<'a>(observations: impl IntoIterator<Item = &'a Value>) -> Self {
        let mut s = Self::default();
        for o in observations {
            s.total += 1;
            s.elapsed_ms += elapsed(o);
            match verdict(o) {
                "error" => s.errors += 1,
                "unsure" => {}
                _ => {
                    s.decided += 1;
                    s.correct += u32::from(o["would_keep"] == o["expected_keep"]);
                }
            }
        }

        s
    }

    fn share(&self) -> f64 {
        f64::from(self.decided) / f64::from(self.total)
    }

    fn accuracy(&self) -> f64 {
        f64::from(self.correct) / f64::from(self.decided.max(1))
    }

    fn mean_ms(&self) -> f64 {
        self.elapsed_ms / f64::from(self.total)
    }
}

fn report(runs: &[Runs], doctor: &Value) {
    let totals: Vec<Score> = runs
        .iter()
        .map(|by_task| Score::of(by_task.iter().flatten()))
        .collect();
    let big_ms = totals[BIG].mean_ms();
    println!(
        "| Method | Decided | Accuracy of decided | Correct of all | Mean time | Speed vs 27B |"
    );
    println!("| --- | --- | --- | --- | --- | --- |");
    for ((_, name), s) in METHODS.iter().zip(&totals) {
        println!(
            "| {name} | {:.0}% | {:.1}% | {:.1}% | {:.0} ms | {:.1}x |",
            100.0 * s.share(),
            100.0 * s.accuracy(),
            100.0 * f64::from(s.correct) / f64::from(s.total),
            s.mean_ms(),
            big_ms / s.mean_ms()
        );
    }
    println!("\nPer task, accuracy of decided (decided share):\n");
    println!("| Method | {} |", TASKS.map(|t| t.spec).join(" | "));
    println!("| --- |{}", " --- |".repeat(TASKS.len()));
    for ((_, name), by_task) in METHODS.iter().zip(runs) {
        let cells: Vec<_> = by_task
            .iter()
            .map(Score::of)
            .map(|s| format!("{:.1}% ({:.0}%)", 100.0 * s.accuracy(), 100.0 * s.share()))
            .collect();
        println!("| {name} | {} |", cells.join(" | "));
    }
    // The cascade reuses d3 and big observations, so its errors are already counted.
    let errors: u32 = totals[..ENDPOINTS].iter().map(|s| s.errors).sum();
    println!(
        "\n{} cases per method, {errors} errors. Hardware: {}.",
        totals[D3].total,
        hardware()
    );
    let models: Vec<_> = doctor["endpoints"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            let fingerprint = e["fingerprint"].as_str().unwrap_or_default();
            let file = fingerprint.rsplit('/').next().unwrap_or_default();
            format!("{}={file}", e["name"].as_str().unwrap_or_default())
        })
        .collect();
    println!("Models: {}", models.join(", "));
}

fn hardware() -> String {
    let cpu = fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|info| {
            let line = info.lines().find(|line| line.starts_with("model name"))?;
            Some(line.split_once(':')?.1.trim().to_owned())
        })
        .unwrap_or_else(|| "unknown CPU".into());
    let lspci = Command::new("lspci")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let gpus: Vec<_> = lspci
        .unwrap_or_default()
        .lines()
        .filter(|line| {
            let line = line.to_lowercase();
            ["vga", "display", "3d controller"]
                .iter()
                .any(|kind| line.contains(kind))
        })
        .filter_map(|line| Some(line.split_once(": ")?.1.to_owned()))
        .collect();
    let gpu = if gpus.is_empty() {
        "no GPU".into()
    } else {
        gpus.join("; ")
    };
    let layers = std::env::var("EIGHTBALL_DEV_NGL").unwrap_or_else(|_| "all".into());

    format!("{cpu}, {gpu}, GPU offload {layers} layers")
}
