# Benchmark

We compare every model of the local stack with Qwen3.8 27B, a large local model, on the same questions. All models answer through `8ball evaluate`, with the same spec text and the default bands, without calibration. Each answer is a new model call.

| Method | Decided | Wrong | Accuracy of decided | Correct of all | Correct vs 27B (95% CI) | Accuracy at 90% coverage | Mean time | Speed vs 27B |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| vllm-d3-nano (systemone) | 91% | 27 | 97.3% | 88.6% | -5.1 (-6.9 to -3.3) | 97.4% | 84 ms | 15.2x |
| bge-reranker-v2-m3 (rerank) | 0% | 0 | - | 0.0% | -93.7 (-95.1 to -92.1) | 58.2% | 25 ms | 52.1x |
| Qwen3.5 0.8B (logprob) | 30% | 58 | 83.0% | 25.3% | -68.4 (-71.0 to -65.6) | 51.7% | 74 ms | 17.2x |
| Qwen3.8 27B (logprob) | 97% | 32 | 97.0% | 93.7% | reference | 97.8% | 1280 ms | 1.0x |
| d3, then 27B for `unsure` | 99% | 38 | 96.6% | 95.4% | +1.8 (+0.9 to +2.8) | 97.4% | 198 ms | 6.5x |

The columns:

- **Decided**: the share of cases with a `yes` or `no` verdict. The others are `unsure`.
- **Wrong**: decided cases with the wrong verdict.
- **Accuracy of decided**: right verdicts among decided cases. Methods that decide different shares do not compare well on this column, because the cases that a method leaves `unsure` are the hard ones.
- **Correct of all**: right verdicts among all cases. `unsure` counts as not right.
- **Correct vs 27B**: the difference in "correct of all" from the 27B, in points, with a 95% interval from a paired bootstrap. If the interval contains 0, the difference is noise.
- **Accuracy at 90% coverage**: each method decides the 90% of cases that it is most sure of, by its larger probability. This compares the models at the same coverage.

On the cases that it decides, d3 is as accurate as the 27B (97.3% and 97.0%), and it is 15 times faster. d3 is `unsure` for 9% of the cases. If the 27B answers those cases, 1.8 points more of all cases get a right verdict than with the 27B alone (95% interval: 0.9 to 2.8 points), 6.5 times faster.

| Method | spam | positive | lost-card |
| --- | --- | --- | --- |
| vllm-d3-nano (systemone) | 99.2% (98%) | 93.8% (84%) | 99.0% (91%) |
| bge-reranker-v2-m3 (rerank) | - (0%) | - (0%) | - (0%) |
| Qwen3.5 0.8B (logprob) | 88.5% (6%) | 77.8% (56%) | 94.4% (28%) |
| Qwen3.8 27B (logprob) | 97.7% (98%) | 96.3% (95%) | 97.1% (97%) |
| d3, then 27B for `unsure` | 98.2% (99%) | 93.4% (98%) | 98.4% (99%) |

Each cell is the accuracy of decided cases, with the decided share in brackets.

The cases:

| Spec | Question | Dataset | Cases (yes / no) |
| --- | --- | --- | --- |
| `spam` | Is this text message spam? | [SMS Spam Collection](https://huggingface.co/datasets/ucirvine/sms_spam), train | 200 / 200 |
| `positive` | Is this movie review positive? | [SST-2](https://huggingface.co/datasets/stanfordnlp/sst2), validation | 200 / 200 |
| `lost-card` | Is the customer asking about a lost, stolen, or compromised card, including fraud or someone else using their card? | [Banking77](https://huggingface.co/datasets/PolyAI/banking77), train and test, classes 22 and 41 | 119 / 200 |

How we keep it fair:

- The labels come from public datasets. We did not write or label the cases.
- The benchmark pins each dataset revision. The SHA-256 of each text puts the case in the `test` or the `dev` split, and orders the cases. So no person picks them.
- We change questions only from results on `dev`, and we report only `test`. On `dev`, the first `lost-card` question ("Does the customer report a lost, stolen, or compromised card?") missed fraud cases, which Banking77 labels `compromised_card`. The current question halved the 27B's misses on `dev` (8 to 4 of 200), and kept d3 at 4.
- Every model gets the same question. No model gets a prompt that is tuned for it.
- We show all models, also those that do badly.

On 40 SST-2 cases, d3 on 4 CPU cores gave the same 40 verdicts as on the GPU, and P(yes) changed by at most 0.02. The [CI comparison](#ci) covers 600 cases and two models. Time depends on your hardware. The speed ratio is more stable, because all models run on the same machine. We measured on an AMD Ryzen AI 9 HX 370 with its Radeon 890M iGPU (Vulkan, all layers on the GPU), on 2026-11-10.

Limits:

- The reranker answers `unsure` without calibration, because its score shows topic match, not the answer. Its row shows what it would answer at 90% coverage.
- Qwen3.5 0.8B is `unsure` for most cases with the default bands. Calibration can change this.
- The cases are short texts, and there are 3 tasks. Long states and other questions can give other results.
- The numbers come from 1 run. The 95% intervals show what is noise: about 1 to 2 points at this size.

To run it yourself (the 27B model is a 19 GB download):

```sh
dev/models.sh up
dev/models.sh bench
cargo build --release -p cli -p bench
cargo run --release -p bench                 # test split, 200 cases per answer and task
cargo run --release -p bench -- --split dev  # to try a question change
```

The `bench` crate prints these tables, with your hardware and the model files. It uses the `8ball` binary from the same build. It saves each answer, with its text, to `target/bench/observations.json`, the counts per task to `target/bench/summary.json`, and the tables for the published page to `target/bench/page.md`. `cargo run -p bench -- --help` lists the options: other endpoints, a cascade, extra configuration, and `--baseline` to compare with an earlier `summary.json`.

## Decider Flash v1.3

Since 2026-11-23 the deployed d3 Service runs Perplexity Decider Flash v1.3 (Q4_K_M). The tables above are from vllm-d3-nano, the model before it. CI keeps vllm-d3-nano, because Flash is too slow on the CPU of the runners (see below).

On the deployed RTX 2070, Flash answered the 600 test cases (100 per side) with 0 errors, on 2026-11-27:

| Model | Decided | Wrong | Accuracy of decided | Correct of all | Mean time |
| --- | --- | --- | --- | --- | --- |
| Decider Flash v1.3 Q4_K_M, RTX 2070 | 575 (96%) | 8 | 98.6% | 567 (94.5%) | 85 ms |
| vllm-d3-nano Q8_0, CI CPU baseline | 543 (90.5%) | 10 | 98.2% | 533 (88.8%) | - |

Flash decides more cases and gets fewer wrong. Per task (decided, wrong): spam 199, 0; positive 183, 5; lost-card 193, 3. The mean time includes a `kubectl port-forward` from a laptop.

On 4 CPU threads, Flash took 1.3 to 1.6 times as long per case as nano (2380 and 2375 ms against 1785 and 1507 ms, 120 cases in two runs). We tried Flash in CI ([#103](https://github.com/Tanguille/8-ball/pull/103)). On a slow runner the live tests took 968 s instead of nano's 555 s median. The 64-item triage took 225 s of its 240 s limit, and a 2,000-character state exceeded the 20 s backend timeout. On a faster runner, one benchmark case timed out. So CI keeps nano, and Flash is checked against the deployed GPU by hand. To repeat that check, run `kubectl -n ai port-forward svc/d3-nano 19931:8080`. Then put `[endpoints.flash]` with `url = "http://127.0.0.1:19931"` in a TOML file, and run `cargo run --release -p bench -- --config flash.toml --endpoints flash --per-side 100`. Without `--endpoints`, the bench runs its default local endpoints, and stops when they are not up.

## D3 v3.1 on llama.cpp

The pinned D3 file, [conversion `50dea20`](https://huggingface.co/ggml-org/vllm-d3-nano-GGUF/commit/50dea2057df10c4b393559a155ab3bf0bc7532c6), already has the [v3.1.0 weights](https://huggingface.co/vllm-sr/d3-nano/commits/462efc366229ea268cf4eed3738b3fc4415e4817), and the newer [conversion `f2256df`](https://huggingface.co/ggml-org/vllm-d3-nano-GGUF/commit/f2256dfaff5b8337e8bf67ca064aeef18ee14a79) gave the same verdicts and probabilities on 600 test cases, so we kept the pin.

## Service fallback trial

Keep D3 alone for spam, sentiment, and lost-card: sending its `unsure` cases to Qwen3.8 27B gained about 7 points of correct answers, but sentiment and lost-card went over the 1% error limit on hosted calls and got more wrong answers ([paired hosted run](https://github.com/Tanguille/8-ball/actions/runs/44940853914), [timeout follow-up](https://github.com/Tanguille/8-ball/actions/runs/44950702256)).

## Latest results

The local-stack tables at the top of this page come from one run by hand. CI runs the benchmark again and saves the tables on the `bench-results` branch. Each page shows the date, the 8-ball commit, the model files, and the hardware.

| Page | Models | Where they run | When |
| --- | --- | --- | --- |
| [cpu](https://github.com/Tanguille/8-ball/blob/bench-results/cpu.md) | d3, the reranker, Qwen3.5 0.8B | The CPU of the GitHub runner | Every push to `main` |
| [27b](https://github.com/Tanguille/8-ball/blob/bench-results/27b.md) | d3, Qwen3.8 27B, the cascade | d3 on the runner, the 27B at a hosted provider | Every night but Sunday |
| [matrix](https://github.com/Tanguille/8-ball/blob/bench-results/matrix.md) | d3 and every hosted model that can answer a decision question | d3 on the runner, the others at a hosted provider | Sunday |

Compare accuracy between pages only when the cases per side are the same. Do not compare time between a hosted model and a local model, because the hosted time includes the network. A model with errors on more than 1% of its cases is left off the page, and the page names it at the top. A page is not replaced when d3 is excluded or fewer than 2 methods remain, so a page can be older than the last run. The history of the branch has the earlier results.

## CI

| Workflow | When | What |
| --- | --- | --- |
| `ci.yaml` | Every pull request and main push | `cargo fmt`, `clippy` and `nextest`. Two model jobs run in parallel on the pinned models: `live` runs the live tests, and `bench` runs a CPU benchmark. Pull requests limited to Markdown, `.mise.toml`, or the lint workflow skip both model jobs. |
| `bench-hosted.yaml` | Main pushes, every night but Sunday, and by hand | d3, Qwen3.8 27B on a hosted provider, and the cascade |
| `bench-hosted.yaml` | Sunday, and by hand | Every hosted model that can answer a decision question, from its live catalog |

The skip avoids the model jobs for changes that only affect documentation or lint tooling. Two lint-tool update PRs spent 29 and 33 minutes in the model job before the skip. Main pushes, manual runs, and PRs whose changes cannot be checked still run it. [Markdown lint update](https://github.com/Tanguille/8-ball/actions/runs/44696028191), [Shell lint update](https://github.com/Tanguille/8-ball/actions/runs/44545507966)

A pull request benchmarks only d3. The reranker never decides before calibration, and Qwen3.5 0.8B only reports, so main pushes and runs by hand add them. The benchmark fails when a task of d3 or the reranker gets worse than `bench/baseline.json` by more than 1% of its cases. CPU and GPU builds can flip a borderline verdict, so 1% is allowed. On the same 600 cases, d3 gave the same verdicts on CPU and GPU, and Qwen3.5 0.8B gave 20 other verdicts. So Qwen3.5 0.8B changes are shown, but do not fail the run. When a pull request changes verdicts on purpose, copy the `d3` block of `summary.json` from the job's `bench` artifact into `bench/baseline.json` in the same pull request. Keep the `rerank` and `chat` blocks of the baseline, because the pull request's summary has no rows for them. For a `summary.json` with every endpoint, run CI by hand on the branch.

The hosted runs need the `HOSTED_API_KEY` repository secret and the `HOSTED_URL` repository variable. The optional `HOSTED_SKIP` variable is a regex for model ids to leave out. Its default, `^8ball$|/`, skips our own joke model and router ids that contain a slash. The report counts the models that cannot answer and does not name them. The 27B run fails on a verdict regression or if any endpoint has more than 1% call errors. If the provider is down (the model list fails, no model is listed, or the doctor probe fails three times), the run is skipped with a warning, and it saves and publishes nothing. A failed run keeps its report and observations for review. Only a passing run replaces the baseline. The first passing run under this rule starts a fresh baseline, since older runs could save one despite excessive errors. After an intended verdict change, run the workflow by hand with `accept` to skip the comparison. The call error limit still applies. The weekly matrix only warns on regressions. Pull requests never get the secret. `dev/hosted.sh` prints the hosted endpoints that a run uses.

The `publish` job of each workflow writes the pages. It runs only on `main`, never for a pull request, and it has no secret. In CI it runs for pushes. The hosted workflow also runs it for schedules and runs by hand. Create the `bench-results` branch once, with `git switch --orphan bench-results && git commit --allow-empty -m "chore: start" && git push origin bench-results`.
