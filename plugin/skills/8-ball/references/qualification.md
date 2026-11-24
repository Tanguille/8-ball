# Qualify a recurring decision

## Agree on what a good answer means

Ask the user what they will do with each answer, which mistake costs more, and what would make the filter worth using. Turn words like “quality” or “urgent” into a question someone can answer from the evidence available at decision time.

Give the user a short batch of real items to label **before showing any predictions**. Include ordinary cases, close calls, and something they would hate to lose. Ask them to fill in the last two columns. Leave a case open if they need more evidence.

| Item and evidence | Your answer | What tipped it for you? |
| --- | --- | --- |
| … | | |

For a domain shortlist, try a pair like `h4ck.example` and `account-update.example`. Ask “Which would you save for a playful personal site, and why?” An unconventional spelling might be the whole appeal. If the user values the wordplay, test another playful name and a look-alike they find dull. Let them supply both labels before writing that distinction into the question.

Adapt the exercise to the decision:

- **Filter** (`noul`). Ask for a must-keep item that looks skippable and a near match they would skip. Agree on how many missed keepers they can tolerate.
- **Route or tag** (`choice`). Define each destination with the user. Ask where a ticket that fits two teams should go. If several tags can apply at once, ask a separate yes/no question for each tag.
- **Rate** (`score`). Ask for examples on either side of a level boundary, such as “today” versus “now.” Have the user order the levels and say which boundary changes the action.

Read the resulting rule back with one example on each side of the hardest boundary. Wait for the user to correct or confirm it. If the task depends on taste and the user has supplied no labels, leave the rule as a draft.

## Try the rule alongside the usual work

Set aside fresh, representative cases for a final test. Keep duplicates and variants of the same event together so they cannot appear on both sides of the split. Keep expected answers outside the model input.

On the remaining cases, run the question with `learn: false` to exclude remembered labels and exploration. For triage, omit `top_k` so every item gets judged. Keep processing all items as usual. Show the user the evidence, their label and the verdict side by side for disagreements, confident rejections and abstentions. Ask what explains the difference. Change the wording, options, or evidence only when their explanation calls for it, then retry the development cases.

Once the user accepts the wording, use a named spec (`[specs.<name>]`) for repeated runs. If fitting thresholds, use `decision_review` to get unlabeled spec states without predictions. Send confirmed development labels with `decision_label` using the saved row id and `yes`/`no`, the option name, or the score index. A saved label can become the answer the next time that state appears. Keep unresolved and held-out cases out of this step.

Preview a fit with `8ball calibrate <spec> --dry-run`. At the default 0.95 precision target a side needs at least 20 decided labels. Check the counts and warnings. If fitting is appropriate, save the fit with `8ball calibrate <spec>` on the same server that received the labels; if that is a separate test server, point the CLI at it with `--url` and set `recalibrate_after = 0` in its config. Calibration changes which answers are decisive. A fit cannot teach the model what the user finds funny.

## Test the frozen rule and decide together

Keep the spec, endpoint, model and thresholds fixed for the final test. Record them with the results. Disable automatic refits in the test setup (`recalibrate_after = 0`) or keep new labels out during the comparison.

For keep/skip, replay independently labeled JSONL with a unique id per row. Include the original `context` if the decision used it. For example, after a reviewer chose to keep a particular item:

```json
{"id":"holdout-17","item":"punjar.example","expected_keep":true}
```

Run `8ball evaluate --spec <spec> < holdout.jsonl`. Add `--keep` with the desired option or minimum level for a choice or score. For exact routing or ratings, compare each verdict with the user's exact option or level too. With MCP alone, replay the held-out inputs with `learn: false` and tally the same outcomes against the private answers.

Show how many useful items would be lost, how many unwanted ones would pass, and how many cases were decisive, unsure, or errors out of the total. For example, “2 of your 18 keepers would disappear” gives the user something concrete to judge. Let them inspect those items. If the results lead to another revision, move the inspected cases into development and test on fresh cases.

Ask the user to choose whether to use the rule for the stated action, keep it advisory, or revise it. Until they choose, keep the usual workflow. In use, send unsure cases and errors through that workflow and periodically have the user blind-review a sample of skips as well as keeps. Recheck the rule when the model, question, input mix, or user's preferences change.
