# Calibration and learning

Bands select the decisive verdicts at the requested precision. The other verdicts are `unsure`.

For each side, calibration picks the loosest threshold whose labeled answers still reach the target precision (`--precision`, default 0.95). So calibration makes a question decide more cases at that precision. It does not make verdicts safer than the default bands. If the defaults are already more precise than the target, calibration gives lower precision for more coverage. To keep high precision, ask for a high target.

A reranker answers `unsure` until you calibrate the question, because its score shows topic match, not the answer.

To calibrate a spec:

1. Save the question as a reviewed spec in the configuration.
2. Label real decisions on each side. A side needs at least 1 / (1 − precision) decided labels: 20 for 0.95, 50 for 0.98, and 100 for 0.99. With fewer, a first calibration stops that side from deciding, and a refit keeps its previous threshold. `8ball decisions --spec <spec> --unlabelled` lists them, and `8ball label <id> <value>` saves a label. An agent can do the same with `decision_review` and `decision_label`. Label only from evidence, such as the outcome of the work.
3. Run `8ball calibrate <spec>`.

Reports are in-sample, so evaluate a separate holdout (labeled cases that calibration did not use). A change to the question or the model file invalidates its calibration identity. A new llama.cpp build of the same file does not. 8-ball checks the model again every minute. Ad-hoc questions learn by their exact text when `learning.ad_hoc` is on. They get their own calibration, apart from specs.

8-ball also refits a question by itself at every multiple of `recalibrate_after` labels. This refit uses only cached answers, so it makes no model calls. A side without enough evidence keeps its threshold. Manual `8ball calibrate` still works for specs. Keep `recalibrate_after = 0` during a holdout comparison, so that the calibration stays fixed.

## Learning upgrade notes

- With the defaults, about `explore` (5%) of skipped answers come back as `unsure`. This is extra work by design. `report = false` also stops exploration.
- Metrics count explored answers as `unsure`, and remembered answers as `cached`. The `questions` and `decisions` listings show the stored model verdict, so an explored answer keeps its model verdict there.
- A label becomes the answer for that exact question and state. Anyone with the API key can set it. To fix a wrong label, label the same id again.
- To turn learning off, set `report = false`, `remember = false`, `explore = 0.0`, `recalibrate_after = 0`, and `ad_hoc = false`.
