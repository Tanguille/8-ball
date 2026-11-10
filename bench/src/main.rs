//! Benchmark every model of the local stack on fixed public cases. See README "Benchmark".
//!
//! Usage: `dev/models.sh up && dev/models.sh bench && cargo build --release -p cli -p bench`,
//! then `cargo run --release -p bench -- [--per-side 200] [--split dev]`.

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
use clap::{Parser, ValueEnum};
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
    paths: &'static [&'static str],
    field: &'static str,
    yes: &'static [i64],
}

// Pinned revisions, so that every run judges the same cases.
const TASKS: [Task; 3] = [
    Task {
        spec: "spam",
        repo: "ucirvine/sms_spam",
        sha: "cae486f927c250fe1d4a5b55f11357964ed1646c",
        paths: &["plain_text/train-00000-of-00001.parquet"],
        field: "sms",
        yes: &[1],
    },
    Task {
        spec: "positive",
        repo: "stanfordnlp/sst2",
        sha: "8d51e7e4887a4caaa95b3fbebbf53c0490b58bbb",
        paths: &["data/validation-00000-of-00001.parquet"],
        field: "sentence",
        yes: &[1],
    },
    Task {
        spec: "lost-card",
        repo: "PolyAI/banking77",
        sha: "1846984189f2bc0ac51c6fc3a600fa9e1714e0c6",
        // The test file alone has only 80 cases with the answer yes.
        paths: &[
            "data/train-00000-of-00001.parquet",
            "data/test-00000-of-00001.parquet",
        ],
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

/// Each case belongs to one split by its id, so the two never share a case. Tune questions and
/// policies on `dev`; report only `test`.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Split {
    Test,
    Dev,
}

#[derive(Parser)]
struct Args {
    /// Cases per expected answer, per task. A task with fewer uses all it has.
    #[arg(long, default_value_t = 200)]
    per_side: usize,
    #[arg(long, value_enum, default_value_t = Split::Test)]
    split: Split,
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
        let cases = cases(task, args.per_side, args.split, &args.out).await?;
        for case in &cases {
            writeln!(lines, "{case}")?;
        }
        for ((endpoint, _), by_task) in METHODS[..ENDPOINTS].iter().zip(&mut runs) {
            eprintln!("{} / {endpoint} ...", task.spec);
            let mut observations = evaluate(task.spec, endpoint, &lines)?;
            // Keep the text with each answer, so misses can be read in observations.json.
            for (o, case) in observations.iter_mut().zip(&cases) {
                o["item"] = case["item"].clone();
            }
            by_task.push(observations);
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

async fn cases(task: &Task, per_side: usize, split: Split, cache: &Path) -> Result<Vec<Value>> {
    // SMS spam repeats some messages, so keep one case per trimmed text. The last row wins.
    let mut rows: HashMap<String, (String, bool)> = HashMap::new();
    for path in task.paths {
        read_rows(task, &download(task, path, cache).await?, &mut rows)?;
    }
    // The id hashes the raw text; sorting by it gives a fixed sample that no one picked by hand.
    let mut all: Vec<_> = rows
        .into_iter()
        .map(|(item, (raw, keep))| (domain::verdict::short_hash(&raw), item, keep))
        .filter(|(id, _, _)| is_test(id) == (split == Split::Test))
        .collect();
    all.sort_unstable();
    let side = |keep: bool| all.iter().filter(move |c| c.2 == keep).take(per_side);

    Ok(side(true)
        .chain(side(false))
        .map(|(id, item, keep)| json!({"id": id, "item": item, "expected_keep": keep}))
        .collect())
}

/// Half of the ids start with an even hex digit: those cases form the test split.
fn is_test(id: &str) -> bool {
    id.chars()
        .next()
        .and_then(|c| c.to_digit(16))
        .is_some_and(|digit| digit % 2 == 0)
}

async fn download(task: &Task, path: &str, cache: &Path) -> Result<PathBuf> {
    let file = cache.join(format!(
        "{}-{}-{}",
        task.repo.replace('/', "__"),
        &task.sha[..12],
        path.replace('/', "__")
    ));
    if !file.exists() {
        let url = format!(
            "https://huggingface.co/datasets/{}/resolve/{}/{path}",
            task.repo, task.sha
        );
        fs::write(
            &file,
            reqwest::get(url).await?.error_for_status()?.bytes().await?,
        )?;
    }

    Ok(file)
}

fn read_rows(task: &Task, file: &Path, rows: &mut HashMap<String, (String, bool)>) -> Result<()> {
    for row in SerializedFileReader::new(fs::File::open(file)?)?.get_row_iter(None)? {
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

    Ok(())
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

    /// "-" when nothing was decided, so 0 of 0 does not read as 0% accurate.
    fn accuracy_text(&self) -> String {
        if self.decided == 0 {
            "-".into()
        } else {
            format!(
                "{:.1}%",
                100.0 * f64::from(self.correct) / f64::from(self.decided)
            )
        }
    }

    fn mean_ms(&self) -> f64 {
        self.elapsed_ms / f64::from(self.total)
    }
}

/// Decided and right. `unsure` and errors count as not correct.
fn correct(o: &Value) -> bool {
    !matches!(verdict(o), "unsure" | "error") && o["would_keep"] == o["expected_keep"]
}

/// Mean of `first - second` over paired cases, with a 95% bootstrap interval.
// ponytail: xorshift instead of a rand dependency; 2000 resamples are plenty for a 95% interval.
fn paired_difference(first: &[bool], second: &[bool]) -> (f64, f64, f64) {
    let n = first.len();
    let diff = |pick: &mut dyn FnMut() -> usize| {
        let sum: i32 = (0..n)
            .map(|_| {
                let i = pick();
                i32::from(first[i]) - i32::from(second[i])
            })
            .sum();
        f64::from(sum) / f64::from(u32::try_from(n).expect("case count fits u32"))
    };
    let mut index = 0;
    let observed = diff(&mut || {
        index += 1;
        index - 1
    });
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut samples: Vec<f64> = (0..2000)
        .map(|_| {
            diff(&mut || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                usize::try_from(state % n as u64).expect("index fits usize")
            })
        })
        .collect();
    samples.sort_by(f64::total_cmp);

    (observed, samples[50], samples[1949])
}

/// Accuracy on the `percent` of cases the method is most sure of, deciding every one of
/// them by the larger probability. Methods with other bands then compare at the same coverage.
fn accuracy_at(observations: &[&Value], percent: usize) -> f64 {
    let mut ranked: Vec<(f64, bool)> = observations
        .iter()
        .map(|o| {
            let yes = o["decision"]["probabilities"]["yes"].as_f64();
            // An error ranks last and counts as wrong.
            yes.map_or((-1.0, false), |yes| {
                (
                    yes.max(1.0 - yes),
                    (yes >= 0.5) == o["expected_keep"].as_bool().unwrap_or_default(),
                )
            })
        })
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    let kept = (ranked.len() * percent).div_ceil(100);
    let right = ranked.iter().take(kept).filter(|(_, right)| *right).count();
    let count = |n: usize| f64::from(u32::try_from(n).expect("count fits u32"));

    count(right) / count(kept)
}

fn report(runs: &[Runs], doctor: &Value) {
    let pooled: Vec<Vec<&Value>> = runs
        .iter()
        .map(|by_task| by_task.iter().flatten().collect())
        .collect();
    let totals: Vec<Score> = pooled
        .iter()
        .map(|obs| Score::of(obs.iter().copied()))
        .collect();
    let flags: Vec<Vec<bool>> = pooled
        .iter()
        .map(|obs| obs.iter().map(|o| correct(o)).collect())
        .collect();
    let big_ms = totals[BIG].mean_ms();
    println!(
        "| Method | Decided | Wrong | Accuracy of decided | Correct of all | Correct vs 27B (95% CI) | Accuracy at 90% coverage | Mean time | Speed vs 27B |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    for (index, ((_, name), s)) in METHODS.iter().zip(&totals).enumerate() {
        let versus = if index == BIG {
            "baseline".to_owned()
        } else {
            let (mean, low, high) = paired_difference(&flags[index], &flags[BIG]);
            format!(
                "{:+.1} ({:+.1} to {:+.1})",
                100.0 * mean,
                100.0 * low,
                100.0 * high
            )
        };
        println!(
            "| {name} | {:.0}% | {} | {} | {:.1}% | {versus} | {:.1}% | {:.0} ms | {:.1}x |",
            100.0 * s.share(),
            s.decided - s.correct,
            s.accuracy_text(),
            100.0 * f64::from(s.correct) / f64::from(s.total),
            100.0 * accuracy_at(&pooled[index], 90),
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
            .map(|s| format!("{} ({:.0}%)", s.accuracy_text(), 100.0 * s.share()))
            .collect();
        println!("| {name} | {} |", cells.join(" | "));
    }
    let sizes: Vec<_> = runs[D3]
        .iter()
        .zip(&TASKS)
        .map(|(obs, t)| {
            let yes = obs.iter().filter(|o| o["expected_keep"] == true).count();
            format!("{} {yes}/{}", t.spec, obs.len() - yes)
        })
        .collect();
    // The cascade reuses d3 and big observations, so its errors are already counted.
    let errors: u32 = totals[..ENDPOINTS].iter().map(|s| s.errors).sum();
    println!(
        "\n{} cases per method (yes/no: {}), {errors} errors. Hardware: {}.",
        totals[D3].total,
        sizes.join(", "),
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn paired_difference_is_zero_for_equal_methods_and_brackets_a_real_gap() {
        let same = [true, false, true, true];
        assert_eq!(super::paired_difference(&same, &same), (0.0, 0.0, 0.0));
        let better: Vec<_> = (0..200).map(|i| i % 10 != 0).collect();
        let worse: Vec<_> = (0..200).map(|i| i % 5 != 0).collect();
        let (mean, low, high) = super::paired_difference(&better, &worse);
        assert!((mean - 0.1).abs() < 1e-9);
        assert!(low > 0.0 && low < mean && mean < high);
    }

    #[test]
    fn accuracy_at_keeps_the_most_sure_cases() {
        let case = |yes: f64, keep: bool| json!({"decision": {"probabilities": {"yes": yes}}, "expected_keep": keep});
        // Sure and right, sure and right, unsure and wrong, error.
        let cases = [
            case(0.99, true),
            case(0.02, false),
            case(0.55, false),
            json!({"error": "x"}),
        ];
        let refs: Vec<_> = cases.iter().collect();
        assert!((super::accuracy_at(&refs, 50) - 1.0).abs() < 1e-9);
        assert!((super::accuracy_at(&refs, 100) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn splits_are_disjoint_halves() {
        let test: Vec<_> = "0123456789abcdef"
            .chars()
            .map(|c| super::is_test(&c.to_string()))
            .collect();
        assert_eq!(test.iter().filter(|t| **t).count(), 8);
        assert!(super::is_test("a1") && !super::is_test("b1"));
    }
}
