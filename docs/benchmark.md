# Benchmark

We compare every model of the local stack with Qwen3.8 27B, a large local model, on the same questions. All models answer through `8ball evaluate`, with the same spec text and the default bands, without calibration. Each answer is a new model call.

| Method | Decided | Wrong | Accuracy of decided | Correct of all | Correct vs 27B (95% CI) | Accuracy at 90% coverage | Mean time | Speed vs 27B |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| vllm-d3-nano (systemone) | 91% | 27 | 97.3% | 88.6% | -5.1 (-6.9 to -3.3) | 97.4% | 84 ms | 15.2x |
| bge-reranker-v2-m3 (rerank) | 0% | 0 | - | 0.0% | -93.7 (-95.1 to -92.1) | 58.2% | 25 ms | 52.1x |
| Qwen3.5 0.8B (logprob) | 30% | 58 | 83.0% | 25.3% | -68.4 (-71.0 to -65.6) | 51.7% | 74 ms | 17.2x |
| Qwen3.8 27B (logprob) | 97% | 32 | 97.0% | 93.7% | baseline | 97.8% | 1280 ms | 1.0x |
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

Accuracy depends only on the model files, the questions, and the cases. On 40 SST-2 cases, d3 on 4 CPU cores gave the same 40 verdicts as on the GPU, and P(yes) changed by at most 0.02. Time depends on your hardware. The speed ratio is more stable, because all models run on the same machine. We measured on an AMD Ryzen AI 9 HX 370 with its Radeon 890M iGPU (Vulkan, all layers on the GPU), on 2026-11-10.

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

The `bench` crate prints these tables, with your hardware and the model files. It uses the `8ball` binary from the same build. It saves each answer, with its text, to `target/bench/observations.json`, and the counts per task to `target/bench/summary.json`. `cargo run -p bench -- --help` lists the options: other endpoints, a cascade, extra configuration, and `--baseline` to compare with an earlier `summary.json`.

## CI

| Workflow | When | What |
| --- | --- | --- |
| `ci.yaml` | Every pull request and main push | `cargo fmt`, `clippy` and `nextest`. Then the live tests and the benchmark on the runner's CPU, with d3, the reranker and Qwen3.5 0.8B. |
| `bench-hosted.yaml` | Main pushes, every night but Sunday, and by hand | d3, Qwen3.8 27B on a hosted provider, and the cascade |
| `bench-hosted.yaml` | Sunday, and by hand | Every hosted model that can answer a decision question, from its live catalog |

The PR benchmark fails when a task of d3 or the reranker gets worse than `bench/baseline.json` by more than 1% of its cases. CPU and GPU builds can flip a borderline verdict, so 1% is allowed. On the same 600 cases, d3 gave the same verdicts on CPU and GPU, and Qwen3.5 0.8B gave 20 other verdicts. So Qwen3.5 0.8B changes are shown, but do not fail the run. When a pull request changes verdicts on purpose, copy `summary.json` from the job's `bench` artifact to `bench/baseline.json` in the same pull request.

The hosted runs need the `HOSTED_API_KEY` repository secret and the `HOSTED_URL` repository variable. The optional `HOSTED_SKIP` variable is a regex for model ids to leave out. They compare with the last saved run of the same kind. The 27B run fails when it gets worse, and the weekly run only warns. After an intended change, run the workflow by hand with `accept` to save a new baseline. Pull requests never get the secret. `dev/hosted.sh` prints the hosted endpoints that a run uses.
