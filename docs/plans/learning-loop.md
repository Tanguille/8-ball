# Learning Loop Implementation Plan

**Created:** 2026-11-09
**Status:** Approved
**Complexity:** Medium
**Estimated Chunks:** 4
**Branch:** `feat/learning-loop`
**Worktree:** `.claude/worktrees/learning-loop`
**Base:** the tip of the stack #14 → #15 → #16 → #18 (`origin/refactor/workspace` at `aa04b07`). Start work only after that stack has merged into `main`. Then run `git fetch origin && git rebase --onto origin/main aa04b07 feat/learning-loop`, so that only this branch's commits remain.

---

## Overview

When 8-ball answers `unsure`, the agent does the work itself and learns the real answer. Today 8-ball drops that answer. With this plan, the agent reports the answer, 8-ball reuses it for the same input, and it refits the decision thresholds of a question after enough reports. A small share of skipped answers (noul `no`, triage `rejected`) is sent back as `unsure`, so the skipped side gets checked as well. Each part is set in a new `[learning]` config table and can be turned off, and a request can opt out with `"learn": false`.

Evidence (cluster, 11-05 to 11-09): the `gitops-investigate` spec has 0 labels. 40% of ad-hoc questions end `unsure` (108 of 268). Labels were only possible through the CLI or REST.

---

## User Stories

### US-1: Report what the work showed
**As an** agent that uses 8-ball
**I want to** know which decisions to report after I do the work
**So that** 8-ball learns from my answer

#### Acceptance Criteria
- [ ] Each `Decision` has `report: bool`. It is true only on decisions the caller works on itself, so a report comes from real work. It is true when `learning.report` is on, the question is learnable, and the decision is not `remembered`, and one of these holds:
  - the verdict is `unsure`, or the decision was `explored`;
  - in `ask`: a noul verdict `yes`;
  - in `triage`: an item in `kept`.
  A `rejected` item is never reported, and neither is an `ask` choice or score verdict other than `unsure`.
- [ ] A question is learnable when it is a spec, or when it is ad-hoc, `learning.ad_hoc` is on, and its question text is stored.
- [ ] The `decision_ask` and `decision_triage` descriptions and the MCP server instructions say: when a decision has `report: true`, call `decision_label` with its `id` and what the work showed after you do the work. Label only from what the work showed; skip the call otherwise.
- [ ] With `learning.report = false`, every decision has `report: false`, the server instructions leave out the reporting line, and nothing is explored.
- [ ] A request can set `"learn": false` (`AskRequest`, `TriageRequest`, default true). Then remember, explore, and report are all off for that request. `8ball evaluate` sends it, so holdout reports measure the model alone.

### US-2: Reuse a reported answer
**As an** agent
**I want** 8-ball to answer with a reported label when the same input comes back
**So that** the same question is never `unsure` twice

#### Acceptance Criteria
- [ ] With `learning.remember = true`, when a labeled decision exists for the same spec (or ad-hoc), question hash, and state hash, the verdict is that label. The decision has `remembered: true`, `cached: true`, `report: false`, and the `calibrated` value of the normal calibration lookup, which runs first. The answer makes no model call.
- [ ] A label that turns out wrong is fixed by labeling the same `id` again, because `set_label` overwrites.
- [ ] A remembered verdict ignores the model fingerprint, because a label is the real answer, not a model output.
- [ ] With `learning.remember = false`, the decision follows today's path.

### US-3: Check some `no` answers
**As an** operator
**I want** a share of confident `no` answers to come back as `unsure`
**So that** reports also cover the `no` side, and calibration is not biased toward `yes`

#### Acceptance Criteria
- [ ] Exploration acts on the skipped side:
  - in `ask`: a noul verdict `no`;
  - in `triage`: an item that would go to `rejected`, of any kind, judged against `keep`.
- [ ] With `learning.explore = p` (0.0 to 1.0), `learning.report` on, and a learnable, not remembered decision, such a decision comes back as `unsure` with `explored: true` and `report: true` when `explore_draw(question_hash, state_hash) < p`. In triage, it lands in `unsure` instead of `rejected`.
- [ ] The draw is deterministic. The same question and state always give the same result. The stored row keeps the model's own verdict.
- [ ] `explore = 0.0`, `report = false`, or `"learn": false` never explores.

### US-4: Refit thresholds automatically
**As an** operator
**I want** 8-ball to recalibrate a question after enough new labels
**So that** nobody needs to run `8ball calibrate` by hand

#### Acceptance Criteria
- [ ] With `learning.recalibrate_after = n` (n > 0), a successful `label` call refits that question when its labeled row count becomes a multiple of `n`.
- [ ] The automatic refit makes no model calls. It uses only labeled states whose probabilities are cached under the current `calibration_id`, and skips the others.
- [ ] Only one automatic refit runs at a time. A `label` call that finds one running skips its own refit.
- [ ] Per side, the merged band is the new threshold if it decides, else the previous one if that decides, else `Bands::default_for`. The merge happens inside `verdict::calibrate`, before the report counts decided and correct examples, so the stored report describes the stored bands.
- [ ] When the merged bands equal the previous bands, nothing is stored, so `calibrated` does not flip on an empty fit.
- [ ] `recalibrate_after = 0` turns this off. Manual `8ball calibrate` keeps today's behavior (no merge, and it re-asks uncached states).

### US-5: Learn ad-hoc questions
**As an** agent that asks the same ad-hoc question often
**I want** labels and calibration for it without first saving a spec
**So that** recurring ad-hoc questions become certain too

#### Acceptance Criteria
- [ ] With `learning.ad_hoc = true`, `label` accepts an ad-hoc decision when its question text is stored (the `questions` table from #16). It checks the value against that question's labels.
- [ ] Without stored question text, `label` rejects the ad-hoc decision with `question text unknown: ask it again first`.
- [ ] An ad-hoc calibration has its own key, `calibration_key(calibration_id, None) = short_hash("{calibration_id}\nad-hoc")`. A spec keeps its `calibration_id` as the key, so existing spec calibrations stay valid. An ad-hoc question with a spec's text never reads or overwrites the spec's bands.
- [ ] An ad-hoc refit uses the endpoint stored on the labeled decision, not the default endpoint.
- [ ] `judge` looks up a calibration for ad-hoc questions too, when `ad_hoc` is on.
- [ ] `labelable` in `questions` and `decisions` is also true for an ad-hoc question with stored text when `ad_hoc` is on.
- [ ] With `learning.ad_hoc = false`, `label` keeps today's error for ad-hoc decisions.

---

## Technical Analysis

### Data/Schema Changes
None. Labels already live in `decisions.label`. Calibrations are keyed by `calibration_key(calibration_id, spec)`. For a spec, the key is `calibration_id` itself, so stored spec calibrations stay valid. For ad-hoc, it is `short_hash("{calibration_id}\nad-hoc")`, so an ad-hoc question with a spec's text never shares the spec's row. Ad-hoc rows store `calibrations.spec = ''`. The column is `NOT NULL` and only informational.

### New Files
| File | Purpose |
|------|---------|
| `service/tests/learning.rs` | End-to-end tests of report, remember, explore, auto-refit, ad-hoc learning, and `learn: false`, against a wiremock systemone server |
| `docs/plans/learning-loop.md` | This plan |

### Modified Files
| File | Changes |
|------|---------|
| `domain/src/config.rs` | `Learning` struct with defaults; `Config.learning`; validation |
| `domain/src/verdict.rs` | `explore_draw`; `calibration_key`; `calibrate(…, previous: Option<Bands>)` with the per-side merge |
| `domain/tests/config.rs`, `domain/tests/calibrate_math.rs` | Tests for the above |
| `store/src/lib.rs` | `labelled(spec: Option<&str>, …)`; `metadata` also returns `endpoint`; new `remembered` and `labelled_count` |
| `store/tests/store.rs` | The new and changed queries |
| `service/src/lib.rs` | Call sites of the changed store and verdict signatures (Chunk 2); the learning loop (Chunk 3) |
| `service/tests/cache.rs` | `labelled(Some("greeting"), …)` call sites (Chunk 2) |
| `service/src/mcp.rs` | Tool descriptions and conditional server instructions |
| `service/Cargo.toml` | `tracing` (warn on a failed refit) and `tokio` with `sync` (the refit lock) |
| `cli/src/main.rs` | `evaluate` sends `"learn": false`; the `Label` help text mentions ad-hoc |
| Existing test files that fail only because of the new defaults | Set the needed `[learning]` option off in that test's config, or send `"learn": false`. Do not change assertions. |
| `README.md`, `docs/hermes-triage.md` | `[learning]` reference, calibration, MCP tools, decision fields, security and upgrade notes |

### API Changes
| Surface | Change |
|---------|--------|
| `Decision` (REST, MCP, CLI output) | New `report`, `explored`, and `remembered` booleans. They are additive, with `#[serde(default)]`, so older JSON still parses in `8ball evaluate`. |
| `AskRequest`, `TriageRequest` (REST, MCP) | New optional `learn: bool`, default true. With false, remember, explore, and report are off for that request. |
| `POST /v1/label`, MCP `decision_label`, `8ball label` | Accept ad-hoc decision ids when `learning.ad_hoc` is on and the question text is stored |

### Conventions Checklist
- [ ] `docs/rust-style.md`: one `use` per path, tight types, no `mod.rs`, a failing regression test first for each behavior change
- [ ] AGENTS.md checks: `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings`, `cargo nextest run`, and the live tests (`dev/models.sh up`, then `cargo nextest run --test live --run-ignored only --no-capture`)
- [ ] Never loosen a live assertion without asking the user
- [ ] Docs in SimpleEnglish plain mode, short
- [ ] Commits in Commitizen style, staged by path, with no AI attribution

---

## Decisions & Trade-offs

### Decision 1: How exploration picks decisions
**Chosen:** deterministic. `explore_draw = u32::from_str_radix(&short_hash("{question_hash}\n{state_hash}")[..8], 16) / (u32::MAX as f64 + 1.0)`. `short_hash` is 16 lowercase hex characters, and the result lies in [0, 1).
**Rationale:** The same input keeps the same answer, cache hits stay consistent, and tests are exact. A random draw would flip answers and make tests flaky.

### Decision 2: Report and explore follow the side the caller skips
**Chosen:** `judge` only remembers. `ask` and `triage` apply report and explore after `judge`, because only they know which side is skipped:
- `ask`: noul `no` is skipped, noul `yes` is worked, and for choice and score only `unsure` is worked.
- `triage`: `rejected` is skipped, and `kept` and `unsure` are worked.

**Rationale:** A report must come from real work. Exploration must check the side that nobody works. Applying both inside `judge` would report rejected items and explore kept ones when `keep = "no"`.

### Decision 3: The automatic refit
**Chosen:**
- It runs inline in `label`, when the labeled row count becomes a multiple of `recalibrate_after`.
- It uses only probabilities cached under the current `calibration_id`, so it makes no model calls.
- A `tokio::sync::Mutex<()>` taken with `try_lock` makes concurrent labels skip instead of refitting in parallel.

Mark it `// ponytail: inline cached-only refit; move to a background task if it ever needs model calls`.
**Rationale:** `label` stays fast and keeps `&self`. A count that is a multiple of n is easy to explain and never refits on every label. Manual `8ball calibrate` keeps re-asking uncached states.

### Decision 4: Thin evidence never makes a question less certain
**Chosen:** `verdict::calibrate(kind, examples, target, previous: Option<Bands>)`. With `Some(previous)`, it chooses each side before it counts the report:
- the new threshold, if it decides;
- else the previous threshold, if it decides;
- else `Bands::default_for(kind)`.

"Decides" means `0 <= x <= 1`. If the merged noul bands give `yes_at <= no_at`, use `previous`. Manual calibrate passes `None` and behaves as today. The live `fit_fixture` test relies on that.
**Rationale:** Today a side without evidence gets `yes_at = 1.01` or `no_at = -0.01`, which means never decide. An automatic refit must not store that.

### Decision 5: What "remember" matches
**Chosen:** the newest labeled row with the same `spec IS ?`, `question_hash`, and `state_hash`, whatever its `calibration_id`. The row's stored `answer` supplies the probabilities. Calibration re-asks call `judge` with `remember = false`, so a fit always uses model output, never labels.

### Decision 6: Learning can be turned off per request
**Chosen:** `"learn": false` on `ask` and `triage`.
**Rationale:**
- Holdout evaluation must measure the model alone. Remembered labels would make it look perfect, and explored items would blur it.
- Callers that never label can opt out.
- `8ball evaluate` sends `"learn": false`.

### Decision 7: Defaults are on
**Chosen:** `report = true`, `remember = true`, `explore = 0.05`, `recalibrate_after = 20`, `ad_hoc = true`, as the user approved on 2026-11-09. The README gets an upgrade note: about 5% more `unsure` on the skipped side, and `explore = 0.0` turns it off.

---

## Dependencies

### Prerequisites
- [ ] PRs #14, #15, #16, and #18 merged into `main` (shepherded by session 8-ball-25)
- [ ] This branch rebased onto `origin/main`, as in **Base** above

### External Dependencies
- `service/Cargo.toml`: add `tracing = { workspace = true }` and `tokio = { workspace = true, features = ["sync"] }` to `[dependencies]`. Both are already workspace dependencies. No new crates.

---

## Execution Plan

All chunks run one after another in the same worktree. Two chunks in one tree would break each other's builds and race on the git index.

### Progress Tracker

#### Wave 1 (sequential) — Domain
- [x] Chunk 1: `domain`: `Learning`, `explore_draw`, `calibration_key`, merging `calibrate`

#### Wave 2 (sequential) — Store
- [ ] Chunk 2: `store`: `remembered`, `labelled_count`, `labelled(Option)`, `metadata` with endpoint (depends on Chunk 1)

#### Wave 3 (sequential) — Service
- [ ] Chunk 3: `service` and `cli`: the learning loop (depends on Chunks 1 and 2)

#### Wave 4 (sequential) — Docs
- [ ] Chunk 4: README and the Hermes doc (depends on Chunk 3)

### Wave Conflict Matrix
| Chunk | Write Set | Dependencies | Wave |
|-------|-----------|--------------|------|
| 1 | `domain/src/config.rs`, `domain/src/verdict.rs`, `domain/tests/config.rs`, `domain/tests/calibrate_math.rs`, `service/src/lib.rs` (the `verdict::calibrate` call site: pass `None`) | None | 1 |
| 2 | `store/src/lib.rs`, `store/tests/store.rs`, `service/src/lib.rs` (call sites only), `service/tests/cache.rs` | Chunk 1 | 2 |
| 3 | `service/src/lib.rs`, `service/src/mcp.rs`, `service/Cargo.toml`, `service/tests/learning.rs`, `cli/src/main.rs`, existing test configs as needed | Chunks 1, 2 | 3 |
| 4 | `README.md`, `docs/hermes-triage.md` | Chunk 3 | 4 |

Each wave holds one chunk, so no write sets overlap within a wave.

---

### Chunk 1: `domain`: config and verdict helpers
**Status:** Complete
**Wave:** 1
**Estimated Context:** ~35k tokens
**Complexity:** Low

#### Scope
- `domain/src/config.rs`, `domain/src/verdict.rs`, `domain/tests/config.rs`, `domain/tests/calibrate_math.rs`
- `service/src/lib.rs`: only the existing `verdict::calibrate(...)` call, which gains `None`

#### Prerequisites
None (Wave 1)

#### Detailed Steps

1. **Failing tests first**
   - `domain/tests/config.rs`:
     - No `[learning]` gives the defaults in Decision 7.
     - `[learning]\nexplore = 0.0\nad_hoc = false` changes only those two.
     - `explore = 1.5`, `-0.1`, and `nan` fail with `learning.explore must be between 0 and 1`.
     - `[learning]\ncolour = 1` fails (`deny_unknown_fields`).
   - `domain/tests/calibrate_math.rs`:
     - `explore_draw` is deterministic, lies in [0, 1), and differs between 2 states.
     - `calibration_key(id, Some("s")) == id`, and `calibration_key(id, None) != id`.
     - With `previous = Some(Noul{yes_at: 0.85, no_at: 0.15})` and 25 `yes` examples at 0.97 and none `no`, `calibrate` gives `yes_at == 0.97` and `no_at == 0.15`, and its report counts `decided` with those bands.
     - With `previous = Some(Noul{1.01, -0.01})` and no evidence, both sides fall back to `default_for` (0.85 and 0.15).
     - A crossing result returns `previous`.
     - Level: `1.01` with a deciding previous gives the previous value.
     - With `previous = None`, the output is today's, which the existing tests prove.

2. **`Learning` in `domain/src/config.rs`**
   ```rust
   /// How 8-ball learns from answers that agents report. Each part can be turned off.
   #[derive(Debug, Clone, serde::Deserialize)]
   #[serde(deny_unknown_fields, default)]
   pub struct Learning {
       /// Mark decisions whose answer the agent should report with `label`.
       pub report: bool,
       /// Answer with a reported label when the same question and state come back.
       pub remember: bool,
       /// Share of skipped decisions (noul `no`, triage `rejected`) returned as `unsure`, so the agent checks them.
       pub explore: f64,
       /// Refit a question's bands at every multiple of this many labels; 0 turns it off.
       pub recalibrate_after: usize,
       /// Also learn ad-hoc questions, by their exact text.
       pub ad_hoc: bool,
   }
   ```
   - Add `impl Default` with the Decision 7 values, and `#[serde(default)] pub learning: Learning` on `Config`.
   - In `Config::validate`, return `Err("learning.explore must be between 0 and 1".into())` unless `(0.0..=1.0).contains(&self.learning.explore)`. A NaN fails `contains`.

3. **`domain/src/verdict.rs`**
   - Add `pub fn explore_draw(question_hash: &str, state_hash: &str) -> f64`, as in Decision 1.
   - Add `pub fn calibration_key(calibration_id: &str, spec: Option<&str>) -> String`. It returns `calibration_id` for `Some(_)`, else `short_hash(&format!("{calibration_id}\nad-hoc"))`.
   - Add `previous: Option<Bands>` to `calibrate`. After the `bands` are computed and before the decided/correct loop, apply Decision 4 when `previous` is `Some`.
   - Write a small `fn decides(threshold: f64) -> bool { (0.0..=1.0).contains(&threshold) }`.
   - Update the one caller in `service/src/lib.rs` to pass `None`, and the existing `calibrate` calls in `domain/tests/calibrate_math.rs` to pass `None`.

#### Verification
- [ ] `cargo nextest run -p domain` passes. The new tests failed before their code existed.
- [ ] `cargo build --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` are clean.
- [ ] End to end: `8ball config check` on a file with `[learning]\nexplore = 2.0` prints the validation error. `dev/config.toml` prints `ok`.

#### Files Changed Summary
```
feat(domain): add the [learning] config, exploration draws and band merging
```

#### Continuation Prompt
> Continue the Learning Loop plan from `docs/plans/learning-loop.md`.
> Before starting:
> 1. Check that you are in `.claude/worktrees/learning-loop` on branch `feat/learning-loop`
> 2. Run `git log --oneline -5` and `cargo build --workspace`
> Then:
> 3. Mark Chunk 1 as complete in the progress tracker
> 4. Continue with Chunk 2, and follow its steps exactly

---

### Chunk 2: `store`: remembered labels, label counts, decision endpoint
**Status:** Not Started
**Wave:** 2
**Estimated Context:** ~30k tokens
**Complexity:** Low

#### Scope
- `store/src/lib.rs`, `store/tests/store.rs`
- `service/src/lib.rs` and `service/tests/cache.rs`: call sites of the changed signatures only

#### Prerequisites
- Chunk 1

#### Detailed Steps

1. **Failing tests first** (`store/tests/store.rs`)
   - Insert 2 decisions with the same `question_hash` and `state_hash` but different `calibration_id`, and label the older one `no`. Then `remembered(Some("greeting"), "question", state_hash)` returns that row's `(id, answer, label)`. Rows with `spec = NULL` are found only with `None`.
   - `labelled_count(Some("greeting"), "question")` counts only that spec's labeled rows. With `None`, it counts only ad-hoc rows.
   - `labelled(None, hash)` returns ad-hoc labeled rows.
   - `metadata(id)` returns `(spec, question_hash, endpoint)`.

2. **`labelled`**: `spec: &str` → `spec: Option<&str>`, and `WHERE spec=?1` → `WHERE spec IS ?1`. Update the callers: `calibrate` in `service/src/lib.rs` passes `Some(spec)`, and `service/tests/cache.rs` lines with `labelled("greeting", …)` and `labelled("greeting-copy", …)` wrap the spec in `Some(..)`.

3. **`metadata`**: select `spec,question_hash,endpoint`, and return `Option<(Option<String>, String, String)>`. Update the `label` call site in `service/src/lib.rs` to destructure 3 values. It ignores `endpoint` for now.

4. **`remembered`**
   ```rust
   /// The newest labeled decision for this question and state, under any model: a label is the real answer.
   pub async fn remembered(&self, spec: Option<&str>, question_hash: &str, state_hash: &str)
       -> Result<Option<(i64, Probs, String)>, StoreError>
   ```
   SQL: `SELECT id,answer,label FROM decisions WHERE spec IS ?1 AND question_hash=?2 AND state_hash=?3 AND label IS NOT NULL ORDER BY id DESC LIMIT 1`. The partial index `decisions_labelled` covers `(spec, question_hash) WHERE label IS NOT NULL`.

5. **`labelled_count`**
   ```rust
   pub async fn labelled_count(&self, spec: Option<&str>, question_hash: &str) -> Result<usize, StoreError>
   ```
   SQL: `SELECT COUNT(*) FROM decisions WHERE spec IS ?1 AND question_hash=?2 AND label IS NOT NULL`. Convert with `usize::try_from`, as `prune` does.

#### Verification
- [ ] `cargo nextest run -p store` passes. The new tests failed first.
- [ ] `cargo nextest run --workspace` passes, so behavior is unchanged.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` is clean.

#### Files Changed Summary
```
feat(store): find remembered labels, count labels, and return a decision's endpoint
```

#### Continuation Prompt
> Continue the Learning Loop plan from `docs/plans/learning-loop.md`.
> Before starting:
> 1. Check that you are in `.claude/worktrees/learning-loop` on branch `feat/learning-loop`
> 2. Run `git log --oneline -5` and `cargo build --workspace`
> Then:
> 3. Mark Chunk 2 as complete
> 4. Continue with Chunk 3

---

### Chunk 3: `service` and `cli`: the learning loop
**Status:** Not Started
**Wave:** 3
**Estimated Context:** ~95k tokens
**Complexity:** High

#### Scope
- `service/src/lib.rs`, `service/src/mcp.rs`, `service/Cargo.toml`, `service/tests/learning.rs` (new)
- `cli/src/main.rs`
- Existing test configs, only where step 9 says so

#### Prerequisites
- Chunks 1 and 2

#### Detailed Steps

1. **Failing tests first** (`service/tests/learning.rs`)
   - Build the app as `service/tests/review.rs` does. Use a wiremock systemone mock that answers by state:
     - `yes-*` states give `noul = 0.97`;
     - `maybe-*` gives 0.5;
     - `no-*` gives 0.03;
     - a choice question gives option `a` 0.99.

     Use config `[specs.greeting]` (noul) and `[specs.bucket]` (choice `a`, `b`), and a `[learning]` table per test.
   - **report, ask**:
     - `yes-1` and `maybe-1` have `report: true`.
     - `no-1` with `explore = 0.0` has `report: false`.
     - The choice spec's decided `a` has `report: false`.
     - With `report = false`, all of them are false.
   - **report, triage** (`keep = "no"`):
     - Kept `no-*` items have `report: true`.
     - Rejected `yes-*` items have `report: false`.
     - `unsure` items have `report: true`.
   - **remember**:
     1. Insert a labeled decision for `maybe-1` through the store, with a `calibration_id` the service never produces (as `cli/tests/calibrate_loop.rs` does with `"previous-model"`).
     2. Ask `maybe-1`. The verdict is the label, with `remembered: true`, `cached: true`, `report: false`.
     3. The mock gets no new request. Compare `received_requests().len()` before and after.
   - **explore**:
     - With `explore = 1.0`, `ask` of `no-1` gives `unsure`, `explored: true`, `report: true`. A second ask gives the same.
     - With `explore = 1.0` in triage with `keep = "yes"`, `no-*` items land in `unsure`, not `rejected`.
     - With `explore = 0.0` or `report = false`, nothing is explored.
   - **learn: false**: with `explore = 1.0` and a labeled `maybe-1`, a request with `"learn": false` gives the model's verdicts, `remembered: false`, `explored: false`, and `report: false`.
   - **refit**:
     1. Set `recalibrate_after = 25`. Ask 25 `yes-*` states and label each `yes`.
     2. The 25th label stores a calibration with `yes_at == 0.97` and `no_at == 0.15`, and the next ask has `calibrated: true`.
     3. With `recalibrate_after = 25` and only 24 labels, nothing is stored.
   - **calibrate re-ask ignores remember**: label a decision under the fake old `calibration_id`, run `service.calibrate("greeting", None, true)`, and assert that the mock got a re-ask for that state.
   - **ad_hoc**:
     - With `ad_hoc = true`, labeling an ad-hoc decision succeeds.
     - With `ad_hoc = false`, it fails with today's message.
     - With the `questions` row deleted through a raw sqlx connection (as `store/tests/store.rs` does), it fails with `question text unknown`.
     - An ad-hoc question with a spec's text does not get the spec's calibration (`calibrated: false`).
     - `questions` shows the ad-hoc question as `labelable: true`.

2. **`Decision`, `AskRequest`, and `TriageRequest`**
   - On `Decision`, add `pub report: bool`, `pub explored: bool`, and `pub remembered: bool`, each with `#[serde(default)]`. They serialize always.
   - On both requests, add `#[serde(default = "learn_default")] pub learn: bool`, with `fn learn_default() -> bool { true }`, and a doc comment.

3. **`judge`**: add a `remember: bool` parameter. `ask` and `triage` pass `request.learn && self.config.learning.remember`. The calibration re-ask passes `false`.
   - Let `learnable = spec.is_some() || (self.config.learning.ad_hoc && <question text stored>)`.
     - For a spec, that is `true`.
     - For ad-hoc, read `self.store.question(&question_hash)` once per question per call. A question asked through `ask` stores its text on the first fresh decision, so this is false only for questions answered before #16.
   - Calibration lookup:
     - Run it first for every question where `learnable`, with key `verdict::calibration_key(&calibration_id, spec)`.
     - This replaces today's `if spec.is_some()` and its plain `calibration_id`.
     - Use the same key in `calibrate_question` when storing.
   - Per question, choose a source with a small enum, `Source::Remembered(id, probs, label) | Source::Cached(id, probs) | Source::Fresh`:
     - `Remembered` when `remember && learnable` and `self.store.remembered(spec, &question_hash, &state_hash)` returns a row.
     - Else `Cached` when `store.cached(...)` returns a row.
     - Else `Fresh`.
   - Only `Fresh` questions go into the model batch, and only they call `answers.next()`. This keeps the "one probability vector per question" invariant.
   - A `Remembered` decision sets `verdict = label`, `remembered = true`, `cached = true`, and inserts no row.
   - Carry `question_hash`, `state_hash`, and `learnable` on `Judgment`, so `ask` and `triage` can apply step 4 without hashing again.

4. **Report and explore** in `ask` and `triage`, after `judge` and before `persist`
   - Add `fn learn(&self, judgment: &mut Judgment, question: &Question, skipped: bool, worked: bool)`:
     - if `!request.learn || !self.config.learning.report || !judgment.learnable || judgment.decision.remembered`, leave all flags false;
     - else, if `skipped && verdict::explore_draw(&judgment.question_hash, &judgment.state_hash) < learning.explore`, set `verdict = "unsure"`, `explored = true`, `report = true`;
     - else set `report = worked || verdict == "unsure"`.
   - `ask`:
     - noul: `skipped = verdict == "no"` and `worked = verdict == "yes"`;
     - choice and score: `skipped = false` and `worked = false`.
   - `triage`:
     - `skipped = verdict != keep && verdict != "unsure"` and `worked = verdict == keep`;
     - apply this before the kept, rejected, and unsure split, so an explored item lands in `unsure`.
   - The stored `fresh` row keeps the model's verdict, because `persist` stores `fresh` as built in `judge`. Metrics count the returned verdict.

5. **`label`**
   - Destructure `(spec, question_hash, endpoint)` from `metadata`.
   - Ad-hoc (`spec = None`):
     - if `!learning.ad_hoc`, keep today's error;
     - else load the question text with `self.store.question(&question_hash)`; `None` gives `BadRequest("question text unknown: ask it again first")`;
     - parse it as a JSON object, insert `"id": "q"`, and `serde_json::from_value::<Question>`;
     - validate the value as for a spec.
   - After `set_label` succeeds, call `self.maybe_recalibrate(spec.as_deref(), &question, &endpoint).await`. On error, `tracing::warn!(error = %e, "automatic recalibration failed")` and still return the label.

6. **`calibrate_question` and `maybe_recalibrate`**
   - Move today's `calibrate` body into `async fn calibrate_question(&self, spec: Option<&str>, question: &Question, endpoint: &str, precision: Option<f64>, dry_run: bool, auto: bool) -> Result<CalibrateResponse, ServiceError>`:
     - it uses `self.store.labelled(spec, &question_hash)` and the given `endpoint`;
     - when `auto`, it skips labeled states without a cached answer, and re-asks nothing;
     - when not `auto`, it re-asks uncached states through `judge(state, spec, &[question.clone()], endpoint, false)` plus `persist`, not through `ask`.
   - When `auto`, pass `previous = Some(stored bands, or default_for(kind))` to `verdict::calibrate`, and skip `put_calibration` when the merged bands equal `previous`. Manual passes `None`.
   - Store with `put_calibration(&calibration_key(&calibration_id, spec), spec.unwrap_or(""), …)`.
   - Public `calibrate(spec, precision, dry_run)` resolves the spec question and its endpoint as today, then calls `calibrate_question(Some(spec), …, auto = false)`. Callers see no change.
   - `maybe_recalibrate(spec, question, endpoint)`:
     1. Return if `learning.recalibrate_after == 0`.
     2. Return unless `labelled_count(spec, hash) % recalibrate_after == 0`.
     3. Return if `self.refit.try_lock()` fails, where `refit: tokio::sync::Mutex<()>` is a new `Service` field.
     4. Run `calibrate_question(spec, question, endpoint, None, false, true)`.
   - Add the ponytail comment from Decision 3.

7. **`service/src/mcp.rs`**
   - Add to the `decision_ask` and `decision_triage` descriptions: "When a decision has `report: true`, do the work, then call decision_label with its id and what the work showed. Label only from what the work showed; skip it otherwise."
   - `decision_label`: replace "yes or no for a noul spec" with "yes or no for a noul question".
   - `decision_questions`: replace "save them as a spec so they can be calibrated" with "save the frequent ones as a spec, which you can review and name".
   - In `build_instructions`:
     - replace "- To improve a spec, label its decisions with decision_review and decision_label, from evidence only." with "- When a decision has `report: true`, report what the work showed with decision_label.", only when `service.learning().report` is true;
     - keep a decision_review line for reviews.
   - Add `pub(crate) const fn learning(&self) -> &Learning` on `Service`, as `specs()` does.

8. **`labelable` and the CLI**
   - In `questions` and `decisions`, `labelable` is also true for an ad-hoc question with stored text when `learning.ad_hoc` is on.
   - In `cli/src/main.rs`, `evaluate` adds `"learn": false` to its triage body, and the `Label` help says "a saved decision (a current spec, or an ad-hoc question when learning.ad_hoc is on)".

9. **Existing tests**
   - Run `cargo nextest run --workspace`. A test may fail only because of the new defaults: explore flipped a `no`, a label count hit a refit, or ad-hoc labels are now accepted, as in `cli/tests/calibrate_loop.rs::label_validation_rejects_unknown_adhoc_old_and_invalid_values`. For such a test, add the matching option off (`explore = 0.0`, `recalibrate_after = 0`, `ad_hoc = false`) to its config string, or send `"learn": false`. Do not change an assertion.
   - `service/tests/cache.rs::spec_uses_bands_but_ad_hoc_uses_defaults` must pass unchanged, which proves the separate ad-hoc key.
   - Live tests: if one that asserts a model verdict now gets an explored or remembered answer, set `config.learning.explore = 0.0` in `cli/tests/live.rs::dev_config()`, or send `"learn": false` in that test's request. Leave `dev/config.toml` as is, so the dev server keeps the real defaults. If a live test fails for any other reason, stop and ask the user. Never edit a live assertion.

#### Verification
- [ ] Every test in `service/tests/learning.rs` failed before its step and passes now.
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo nextest run --workspace` pass.
- [ ] Live: `dev/models.sh up`, then `cargo nextest run --test live --run-ignored only --no-capture`: all pass.
- [ ] End to end, with `8ball serve --config dev/config.toml` and the dev models:
  1. Ask a spec state that gives `unsure`; it has `report: true`.
  2. Run `8ball label <id> yes`.
  3. Ask again: `verdict: yes`, `remembered: true`.
  4. `8ball evaluate` on `examples/hermes/holdout.jsonl` reports no remembered or explored effects.
  5. `8ball doctor` reports no errors.

#### Files Changed Summary
```
feat(service): learn from reported answers: remember, explore, refit, ad-hoc labels
```

#### Continuation Prompt
> Continue the Learning Loop plan from `docs/plans/learning-loop.md`.
> Before starting:
> 1. Check that you are in `.claude/worktrees/learning-loop` on branch `feat/learning-loop`
> 2. Run `git log --oneline -5` and `cargo nextest run --workspace`
> Then:
> 3. Mark Chunk 3 as complete
> 4. Continue with Chunk 4

---

### Chunk 4: Docs
**Status:** Not Started
**Wave:** 4
**Estimated Context:** ~25k tokens
**Complexity:** Low

#### Scope
- `README.md`, `docs/hermes-triage.md`

#### Prerequisites
- Chunk 3

#### Detailed Steps

1. **Configuration reference**: add a `[learning]` block to the TOML example, with each key, its default, and a short comment from the `Learning` doc comments.
2. **Calibration**:
   - Replace "Ad-hoc questions always use the default bands." with: ad-hoc questions learn by their exact text when `learning.ad_hoc` is on.
   - Add 2 sentences: 8-ball refits a question at every `recalibrate_after` labels from cached answers, and a side without enough evidence keeps its threshold. Manual `8ball calibrate` still works.
   - Add: keep `recalibrate_after = 0` during a holdout comparison, so the calibration stays fixed.
3. **Commands, routes, and MCP tools**:
   - Update the `label` row, the `decision_label` row, and the request fields (`learn`).
   - Explain `report`, `explored`, and `remembered` in one short paragraph.
4. **Upgrade and security notes**:
   - With the defaults, about `explore` of skipped answers come back as `unsure`, which is extra work by design.
   - Metrics count explored answers as `unsure`, and remembered ones as `cached`.
   - A label becomes the answer for that exact state. Anyone with the API key can set it, and labeling the same id again fixes a wrong label.
5. **docs/hermes-triage.md**:
   - In the observation flow, Hermes reports each candidate that has `report: true` with `decision_label` after it investigates. The label is `yes` if investigating was worthwhile.
   - The holdout section notes that `8ball evaluate` sends `"learn": false`.
6. Keep SimpleEnglish plain mode and short sentences.

#### Verification
- [ ] Every `[learning]` key in the README exists in `Learning` with the same default. Copy the README TOML block into a file and run `8ball config check`: `ok`.
- [ ] No text still says that ad-hoc questions cannot be labeled or always use default bands: `grep -n "default bands\|save it as a spec" README.md docs/*.md`.

#### Files Changed Summary
```
docs: describe the learning loop, its settings, and upgrade notes
```

#### Continuation Prompt
> Continue the Learning Loop plan from `docs/plans/learning-loop.md`.
> Before starting:
> 1. Check that you are in `.claude/worktrees/learning-loop` on branch `feat/learning-loop`
> Then:
> 2. Mark Chunk 4 as complete, and do the final documentation pass from Process Instructions

---

## Testing Strategy

### Unit Tests
- `domain`: `Learning` defaults and validation; `explore_draw`; `calibration_key`; `calibrate` with and without `previous`
- `store`: `remembered`, `labelled_count`, `labelled(None, …)`, `metadata` with endpoint
- `service/tests/learning.rs`: report in ask and triage (including `keep = "no"`), remember, explore, `learn: false`, refit, the calibrate re-ask ignoring remember, ad-hoc labels and keys

### E2E Verification
- [ ] `service/tests/learning.rs` drives the REST and service paths against a mock model.
- [ ] Live suite against `dev/models.sh up`.
- [ ] Manual loop with `8ball serve`, `8ball ask`, and `8ball label`, as in Chunk 3's verification.

---

## Rollback Plan

1. There is no schema change, so nothing needs to be migrated back.
2. To turn the behavior off without a release, set `[learning] report = false`, `remember = false`, `explore = 0.0`, `recalibrate_after = 0`, and `ad_hoc = false`. A single caller can send `"learn": false`.
3. To remove the code, revert the PR. Labels and calibrations stored meanwhile stay valid for manual calibration.

---

## Process Instructions

- After completing each step, update the plan with the current status.
- Pause for user confirmation before proceeding to next step.
- Suggest the prompt for continuing to the next step.
- After the last step, make a final documentation pass. Once the plan's contents are consolidated into existing docs, remove the plan file; if no relevant docs exist, rework the plan into a reference document.

**Important**: Every prompt should verify the branch and worktree before doing any work.

---

## Review Report (2026-11-09)

An automated review (Opus subagent) checked the plan against the code at `aa04b07`. Counts: CRIT 2, HIGH 4, MED 8, LOW 6, AMB 7, PIT 4. Both CRITs and every HIGH and MED were fixed in the plan text above. The LOW, AMB, and PIT items were fixed as noted.

- CRIT-1 [FIXED] Calibration re-asks would be answered by `remember` from stored labels and fit new models on old answers. Now `judge` takes `remember: bool`, and the calibration re-ask passes `false` (Decision 5, Chunk 3 steps 3 and 6). There is a regression test.
- CRIT-2 [FIXED] Report and explore ignored triage `keep`, which reported rejected items and explored kept ones. Both now apply in `ask` and `triage` on the skipped and worked sides (Decision 2, US-1, US-3, Chunk 3 step 4). There are triage tests with `keep = "no"` and a choice spec.
- HIGH-1 [FIXED] Ad-hoc and spec calibrations shared a row. Now `calibration_key` gives ad-hoc its own key, spec keys are unchanged, and `cache.rs::spec_uses_bands_but_ad_hoc_uses_defaults` must pass unchanged.
- HIGH-2 [FIXED] The inline refit could make many model calls and run in parallel. Now it uses cached answers only, at every multiple of n, with a `try_lock` guard (Decision 3).
- HIGH-3 [FIXED] The evaluate path was wrong: the file is `cli/src/evaluate.rs`, and reading explored items breaks `single_decision`. Now `evaluate` sends `"learn": false` (Decision 6), so it needs no parsing change.
- HIGH-4 [FIXED] Chunk 2 broke `service/tests/cache.rs`. That file is now in Chunk 2's write set.
- MED-1 [FIXED] The refit trigger compared unlike counts. It now fires at every multiple of `recalibrate_after` labeled rows.
- MED-2 [FIXED] The report and `calibrated` were wrong after the merge. The merge now runs inside `verdict::calibrate`, before the counts, and nothing is stored when the result equals `previous`. The test uses 25 labels, because the 0.95 target needs at least 20 per side.
- MED-3 [FIXED] `previous` itself could be never-deciding. Each side now falls back from new, to previous, to `default_for`.
- MED-4 [FIXED] Remember and refit would skew holdout evaluation. Now `"learn": false` is used in evaluate, and the README says to keep `recalibrate_after = 0` during comparisons.
- MED-5 [FIXED] The ad-hoc refit endpoint was unspecified. `metadata` now returns `endpoint`, which `label` passes on.
- MED-6 [FIXED] Out-of-date texts are now listed in Chunk 3 steps 7 and 8 and Chunk 4 steps 2 and 3.
- MED-7 [FIXED] Security: the descriptions now say "label only from what the work showed", and the README notes how labels override verdicts and how to fix a bad one.
- MED-8 [FIXED] Parallel chunks shared one worktree. All waves are now sequential.
- LOW-1 [FIXED] Explore is gated on `report`. LOW-2 [FIXED] The README notes the metrics effect. LOW-3 [FIXED] The remember test uses a foreign `calibration_id`, so the plain cache cannot satisfy it. LOW-4 [FIXED] `learnable` needs stored text, so old ad-hoc rows get `report: false`. LOW-5 [FIXED] Chunk 4 runs after Chunk 3. LOW-6 [FIXED] The README has an upgrade note.
- AMB-1 to AMB-7 [FIXED] with the suggested rewrites:
  - the calibration lookup runs first;
  - the stored endpoint is used;
  - the 25-label yes-only test;
  - `short_hash` is 16 hex characters;
  - `ad_hoc = false` is named for `calibrate_loop.rs`;
  - live tests change `dev_config()` or send `"learn": false`, not `dev/config.toml`;
  - `"id"` is inserted before `from_value`.
- PIT-1 to PIT-4 [FIXED]:
  - the getter is `pub(crate) const fn`;
  - `tracing` and `tokio` (sync) are added to `service/Cargo.toml`;
  - the draw divides by `u32::MAX + 1`;
  - a per-question `Source` enum keeps the batch invariant.

No item needs a user decision.
