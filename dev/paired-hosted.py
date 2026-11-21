"""Run either arm of the one-off, paired hosted escalation trial."""
import collections
import contextlib
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import urllib.request

binary = Path("target/release/8ball").absolute()
url = "http://127.0.0.1:8100"
env = {key: val for key, val in os.environ.items() if key != "EIGHTBALL_API_KEY"}
task_order = ("spam", "positive", "lost-card")
original_config = Path("dev/config.toml").read_text() + "\n" + Path("dev/bench.toml").read_text()
original_config += """
[learning]
report = false
remember = false
explore = 0.0
recalibrate_after = 0
ad_hoc = false
"""


def save(name, obj):
    (run_dir / name).write_text(json.dumps(obj, indent=2) + "\n")


@contextlib.contextmanager
def serve(config, database, log):
    with (run_dir / log).open("w") as server_log:
        server = subprocess.Popen([str(binary), "serve", "--config", str(run_dir / config),
                                   "--db", str(run_dir / database), "--listen", "127.0.0.1:8100"],
                                  stdout=server_log, stderr=server_log, env=env)
        try:
            for attempt in range(100):
                if server.poll() is not None:
                    raise RuntimeError(f"server exited, see {log}")
                try:
                    with urllib.request.urlopen(url + "/healthz", timeout=1):
                        break
                except (OSError, urllib.error.URLError):
                    time.sleep(0.1)
            else:
                raise RuntimeError(f"server not ready, see {log}")
            yield
        finally:
            if server.poll() is None:
                server.terminate()
                try:
                    server.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()


def result(row):
    if row["error"]:
        return "error"
    verdict = row["decision"]["verdict"]
    if verdict == "unsure":
        return "unsure"
    assert verdict in ("yes", "no")
    return "correct" if (verdict == "yes") == row["expected_keep"] else "wrong"


def count_results(rows):
    totals = collections.Counter(result(row) for row in rows)
    return {val: totals[val] for val in ("correct", "wrong", "unsure", "error")}


def paired(control, candidate):
    assert [(r["id"], r["expected_keep"]) for r in control] == [(r["id"], r["expected_keep"]) for r in candidate]
    vector = [int(result(b) == "correct") - int(result(a) == "correct") for a, b in zip(control, candidate)]
    n = len(vector)
    state = 0x9E3779B97F4A7C15
    mask = (1 << 64) - 1
    samples = []
    for sample in range(2000):
        total = 0
        for case in range(n):
            state = (state ^ (state << 13)) & mask
            state ^= state >> 7
            state = (state ^ (state << 17)) & mask
            total += vector[state % n]
        samples.append(100 * total / n)
    samples.sort()
    return {"delta_pp": 100 * sum(vector) / n, "lower_pp": samples[50], "upper_pp": samples[1949],
            "improved_cases": vector.count(1), "worsened_cases": vector.count(-1)}


def evaluate(task, rows):
    payload = "".join(
        json.dumps({key: row[key] for key in ("id", "item", "expected_keep")}) + "\n"
        for row in rows
    )
    process = subprocess.run(
        [str(binary), "evaluate", "--spec", task, "--timeout-ms", "95000",
         "--url", url],
        input=payload, text=True, capture_output=True, check=True, env=env, timeout=20000,
    )
    result = json.loads(process.stdout)
    observations = result["observations"]
    pair = lambda row: (row["id"], row["expected_keep"])
    assert [pair(row) for row in observations] == [pair(row) for row in rows]
    for row in observations:
        decision = row.get("decision")
        assert not row["error"] and decision, row["error"]
        assert not any(decision[key] for key in ("cached", "calibrated", "remembered", "explored"))
    return observations


def metrics(endpoint):
    with urllib.request.urlopen(url + "/metrics", timeout=5) as response:
        text = response.read().decode()
    pattern = re.compile(
        r'eightball_escalations_total\{endpoint="' + re.escape(endpoint) +
        r'",outcome="(decided|unsure|error|budget)"\}\s+(\d+)$', re.MULTILINE
    )
    return {"decided": 0, "unsure": 0, "error": 0, "budget": 0} | {
        key: int(val) for key, val in pattern.findall(text)
    }


def p95(rows):
    latency = sorted(row["elapsed_ms"] for row in rows)
    return round(latency[math.ceil(len(latency) * 0.95) - 1], 2)


def primary_verdict(row):
    return row["decision"]["verdict"]


def run():
    source = json.loads(Path("target/paired-input/observations.json").read_text())["d3"]
    assert all(len(source[task]) == 200 for task in task_order)
    source = {task: [row for row in source[task] if primary_verdict(row) == "unsure"] for task in task_order}
    assert sum(map(len, source.values())) == 57
    global run_dir
    run_dir = Path("target/paired-results") / "hosted"
    run_dir.mkdir(parents=True, exist_ok=True)
    config = original_config
    endpoint = "hosted-qwen-3-8-27b"
    found = subprocess.run(["dev/hosted.sh", "30", "qwen-3.8-27b"], check=True,
                           text=True, capture_output=True)
    if f"[endpoints.{endpoint}]" not in found.stdout:
        sys.exit("Hosted provider does not advertise the benchmark model")
    config += "\n" + found.stdout + "\ntimeout_secs = 30\n"
    config += f'\n[escalation]\nendpoint = "{endpoint}"\nper_minute = 30\n'
    (run_dir / "config.toml").write_text(config)
    assert not (run_dir / "service.sqlite").exists()
    observations, counters, backend_failures = {}, {}, {}
    log_offset = 0
    with serve("config.toml", "service.sqlite", "service.log"):
        previous = metrics(endpoint)
        for task in task_order:
            print(f"30-second fallback / {task}", flush=True)
            observations[task] = evaluate(task, source[task])
            current = metrics(endpoint)
            counters[task] = {key: current[key] - previous[key] for key in current}
            previous = current
            # Only export counts. Raw diagnostics can include endpoint details.
            with (run_dir / "service.log").open() as log:
                log.seek(log_offset)
                failures = [line.lower() for line in log if "escalation failed" in line]
                log_offset = log.tell()
            backend_failures[task] = {
                "http_429": sum("429" in line for line in failures),
                "timeout": sum("timed out" in line or "timeout" in line for line in failures),
                "total": len(failures),
            }
    save("observations.json", observations)
    save("metrics.json", counters)
    save("backend-failures.json", backend_failures)

    control = source
    report = {}
    for task in task_order:
        a, b, counts = control[task], observations[task], counters[task]
        attempts = counts["decided"] + counts["unsure"] + counts["error"]
        nonescalated = [(x, y) for x, y in zip(a, b) if not y["decision"]["escalated"]]
        assert all(primary_verdict(x) == primary_verdict(y) for x, y in nonescalated)
        assert sum(y["decision"]["escalated"] for y in b) == counts["decided"] + counts["unsure"]
        assert sum(primary_verdict(x) == "unsure" for x in a) == attempts + counts["budget"]
        assert all(primary_verdict(x) == "unsure" for x, y in zip(a, b) if y["decision"]["escalated"])
        report[task] = {"control": count_results(a), "fallback": count_results(b),
                        "paired": paired(a, b), "metrics": counts, "attempts": attempts,
                        "control_p95_ms": p95(a), "fallback_p95_ms": p95(b),
                        "fallback_on_primary_unsure_p95_ms": p95([y for x, y in zip(a, b) if primary_verdict(x) == "unsure"])}
    total_a = [row for task in task_order for row in control[task]]
    total_b = [row for task in task_order for row in observations[task]]
    report["all"] = {"control": count_results(total_a), "fallback": count_results(total_b),
                     "control_p95_ms": p95(total_a), "fallback_p95_ms": p95(total_b),
                     "paired": paired(total_a, total_b)}
    save("paired.json", report)


if __name__ == "__main__":
    run()
