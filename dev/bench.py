# /// script
# requires-python = ">=3.11"
# dependencies = ["pyarrow"]
# ///
"""Benchmark every model of the local stack on fixed public cases. See README "Benchmark".

Usage: dev/models.sh up && dev/models.sh bench && uv run dev/bench.py [--per-side 100]
"""

import argparse
import hashlib
import json
import os
import platform
import statistics
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

import pyarrow.parquet as pq

ROOT = Path(__file__).resolve().parent.parent
URL = "http://127.0.0.1:8098"
# Pinned revisions, so that every run judges the same cases. `yes` holds the labels where the answer is yes.
TASKS = {
    "spam": {
        "repo": "ucirvine/sms_spam",
        "sha": "cae486f927c250fe1d4a5b55f11357964ed1646c",
        "path": "plain_text/train-00000-of-00001.parquet",
        "field": "sms",
        "yes": {1},
    },
    "positive": {
        "repo": "stanfordnlp/sst2",
        "sha": "8d51e7e4887a4caaa95b3fbebbf53c0490b58bbb",
        "path": "data/validation-00000-of-00001.parquet",
        "field": "sentence",
        "yes": {1},
    },
    "lost-card": {
        "repo": "PolyAI/banking77",
        "sha": "1846984189f2bc0ac51c6fc3a600fa9e1714e0c6",
        "path": "data/test-00000-of-00001.parquet",
        "field": "text",
        "yes": {22, 41},  # compromised_card, lost_or_stolen_card
    },
}
# Endpoints of dev/config.toml and dev/bench.toml; "cascade" is computed from d3 and big.
NAMES = {
    "d3": "vllm-d3-nano (systemone)",
    "rerank": "bge-reranker-v2-m3 (rerank)",
    "chat": "Qwen3.5 0.8B (logprob)",
    "big": "Qwen3.8 27B (logprob)",
    "cascade": "d3, then 27B for unsure",
}
ENDPOINTS = [name for name in NAMES if name != "cascade"]


def cases(task, per_side, cache):
    t = TASKS[task]
    file = cache / f"{t['repo'].replace('/', '__')}-{t['sha'][:12]}.parquet"
    if not file.exists():
        url = f"https://huggingface.co/datasets/{t['repo']}/resolve/{t['sha']}/{t['path']}"
        file.write_bytes(urllib.request.urlopen(url).read())
    # SMS spam repeats some messages, so keep one case per text.
    # The id hashes the raw text, as in the published run; changing it would pick other cases.
    rows = {row[t["field"]].strip(): row for row in pq.read_table(file).to_pylist()}
    all_cases = [
        {
            "id": hashlib.sha256(row[t["field"]].encode()).hexdigest()[:16],
            "item": text,
            "expected_keep": row["label"] in t["yes"],
        }
        for text, row in rows.items()
    ]
    # Order by text hash, not file order: a fixed sample that no one picked by hand.
    all_cases.sort(key=lambda case: case["id"])
    return [c for c in all_cases if c["expected_keep"]][:per_side] + [c for c in all_cases if not c["expected_keep"]][
        :per_side
    ]


def evaluate(spec, endpoint, lines):
    run = subprocess.run(
        ["8ball", "evaluate", "--spec", spec, "--endpoint", endpoint, "--timeout-ms", "120000"],
        check=False,
        input=lines,
        capture_output=True,
        text=True,
    )
    if run.returncode not in (0, 4):
        sys.exit(f"8ball evaluate {spec} {endpoint} failed: {run.stderr}")
    output = json.loads(run.stdout)
    # The answer cache is keyed by model, not endpoint: a cached answer would fake the time.
    if output["report"]["cached"]:
        sys.exit(f"{spec} {endpoint}: cached answers; give each endpoint its own model")
    return output["observations"]


def cascade(first, second):
    """d3 decides; its unsure cases go to the 27B. The time is the sum of both calls."""
    # evaluate keeps the input order, so both lists line up.
    return [
        {**b, "elapsed_ms": a["elapsed_ms"] + b["elapsed_ms"]} if verdict(a) == "unsure" else a
        for a, b in zip(first, second, strict=True)
    ]


def verdict(o):
    return "error" if o["error"] else o["decision"]["verdict"]


def score(observations):
    total = len(observations)
    decided = [o for o in observations if verdict(o) not in ("unsure", "error")]
    correct = sum(o["would_keep"] == o["expected_keep"] for o in decided)
    return {
        "total": total,
        "share": len(decided) / total,
        "accuracy": correct / max(len(decided), 1),
        "correct": correct / total,
        "errors": sum(verdict(o) == "error" for o in observations),
        "mean_ms": statistics.fmean(o["elapsed_ms"] for o in observations),
    }


def hardware():
    cpu = next(
        (
            line.split(":", 1)[1].strip()
            for line in Path("/proc/cpuinfo").read_text().splitlines()
            if line.startswith("model name")
        ),
        platform.processor(),
    )
    lspci = subprocess.run(["lspci"], check=False, capture_output=True, text=True).stdout.splitlines()
    gpu = "; ".join(
        line.split(": ", 1)[1]
        for line in lspci
        if any(kind in line.lower() for kind in ("vga", "display", "3d controller"))
    )
    return f"{cpu}, {gpu or 'no GPU'}, GPU offload {os.environ.get('EIGHTBALL_DEV_NGL', 'all')} layers"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--per-side", type=int, default=100, help="cases per expected answer, per task")
    parser.add_argument("--out", type=Path, default=ROOT / "target" / "bench")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    os.environ["EIGHTBALL_URL"] = URL

    config = (ROOT / "dev" / "config.toml").read_text() + "\n" + (ROOT / "dev" / "bench.toml").read_text()
    raw = {method: {} for method in NAMES}
    with tempfile.TemporaryDirectory() as tmp:
        # A new database for each run, so no answer comes from the cache.
        server = subprocess.Popen(
            [
                "8ball",
                "serve",
                "--config-toml",
                config,
                "--db",
                f"{tmp}/bench.db",
                "--listen",
                URL.removeprefix("http://"),
            ],
            stderr=subprocess.DEVNULL,
        )
        try:
            for _ in range(50):
                try:
                    urllib.request.urlopen(f"{URL}/healthz")
                    break
                except OSError:
                    time.sleep(0.2)
            else:
                sys.exit(f"8ball serve did not start on {URL}")
            doctor = json.loads(subprocess.run(["8ball", "doctor"], check=False, capture_output=True).stdout)
            for task in TASKS:
                lines = "".join(json.dumps(c) + "\n" for c in cases(task, args.per_side, args.out))
                for endpoint in ENDPOINTS:
                    print(f"{task} / {endpoint} ...", file=sys.stderr)
                    raw[endpoint][task] = evaluate(task, endpoint, lines)
                raw["cascade"][task] = cascade(raw["d3"][task], raw["big"][task])
        finally:
            server.terminate()

    (args.out / "observations.json").write_text(json.dumps(raw))
    report(raw, doctor)


def report(raw, doctor):
    totals = {m: score([o for obs in by_task.values() for o in obs]) for m, by_task in raw.items()}
    big_ms = totals["big"]["mean_ms"]
    print("| Method | Decided | Accuracy of decided | Correct of all | Mean time | Speed vs 27B |")
    print("| --- | --- | --- | --- | --- | --- |")
    for m, s in totals.items():
        print(
            f"| {NAMES[m]} | {s['share']:.0%} | {s['accuracy']:.1%} | {s['correct']:.1%} "
            f"| {s['mean_ms']:.0f} ms | {big_ms / s['mean_ms']:.1f}x |"
        )
    print("\nPer task, accuracy of decided (decided share):\n")
    print("| Method | " + " | ".join(TASKS) + " |")
    print("| --- |" + " --- |" * len(TASKS))
    for m in raw:
        cells = (score(raw[m][t]) for t in TASKS)
        print(f"| {NAMES[m]} | " + " | ".join(f"{s['accuracy']:.1%} ({s['share']:.0%})" for s in cells) + " |")
    # The cascade reuses d3 and big observations, so its errors are already counted.
    errors = sum(totals[m]["errors"] for m in ENDPOINTS)
    print(f"\n{totals['d3']['total']} cases per method, {errors} errors. Hardware: {hardware()}.")
    print("Models: " + ", ".join(f"{e['name']}={Path(e['fingerprint'] or '?').name}" for e in doctor["endpoints"]))


if __name__ == "__main__":
    main()
