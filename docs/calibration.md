# Calibration and learning

Bands are score ranges for clear answers. Outside them, 8-ball returns `unsure`. Calibration adjusts the bands using known answers, called labels.

Precision is the share of clear answers on one side that are correct. For each side, calibration picks the cutoff that answers the most labeled cases while meeting the target precision. The cutoff is always the score of a correct case. Set the target with `--precision` (default 0.95).

If the default bands are more precise than the target, calibration can trade some precision for more answers. To keep precision high, set a high target.

## Calibrate a saved question

Each side needs at least 1 / (1 − precision) labeled cases with clear answers. That means 20 for 0.95, 50 for 0.98, and 100 for 0.99. With fewer labels, the first calibration stops that side from giving clear answers. Later calibrations keep that side's previous cutoff.

1. Review the question and save it as a spec in the configuration.
2. List unlabeled decisions with `8ball decisions --spec <spec> --unlabelled`. Save each label with `8ball label <id> <value>`. Use evidence such as the outcome of the work to choose the label. An agent can use `decision_review` and `decision_label`.
3. Run `8ball calibrate <spec>`.

The report uses the calibration cases. For an independent test, use a holdout. It contains labeled cases that calibration did not use.

8-ball checks the model again every minute. If the question or the model file changes, 8-ball stops using the old calibration. A new llama.cpp build keeps the calibration if the model file stays the same. If `learning.ad_hoc` is on, 8-ball keeps separate calibrations for questions asked on the spot, keyed by their exact text.

On 160 separate D3 sentiment development cases, calibration raised the correct count from 122 to 143 and the wrong count from six to nine. It used 240 labeled cases and a 95% target. On the 200-case test split, a wording change lowered the wrong count from 10 to six and the correct count from 152 to 150. We kept the original wording and bands for this spec. Before you accept a calibration, check both counts on unseen cases.

8-ball recalibrates each endpoint when its labeled cases pass a multiple of `recalibrate_after`. It uses saved answers for this step, so it makes no model calls. A side without enough labels keeps its cutoff. You can still run `8ball calibrate` for a spec. During a holdout comparison, set `recalibrate_after = 0` to keep the calibration fixed.

Each endpoint has its own bands and cache. The escalation endpoint uses only the cases where the default endpoint returned `unsure`. A label applies to every endpoint that judged the case. Each endpoint counts only the cases it judged toward `recalibrate_after`. The review tools list each case once.

## Learning upgrade notes

- By default, 8-ball returns about 5% of answers it would skip as `unsure`. This gives the agent a sample to review. Set the share with `explore`. Set `report = false` to stop exploration.
- Metrics count explored answers as `unsure` and remembered answers as `cached`. The `questions` and `decisions` listings show the saved model verdict, even for explored answers.
- A label becomes the answer for that exact question and state. Anyone with the API key can set it. To fix a wrong label, label the same id again.
- To turn learning off, set `report = false`, `remember = false`, `explore = 0.0`, `recalibrate_after = 0`, and `ad_hoc = false`.
