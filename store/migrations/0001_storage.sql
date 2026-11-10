CREATE TABLE IF NOT EXISTS decisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    spec TEXT,
    question_hash TEXT NOT NULL,
    calibration_id TEXT NOT NULL,
    state_hash TEXT NOT NULL,
    state TEXT NOT NULL,
    answer TEXT NOT NULL,
    verdict TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    label TEXT
  );
  CREATE INDEX IF NOT EXISTS decisions_cache ON decisions(calibration_id, state_hash);
  CREATE INDEX IF NOT EXISTS decisions_labelled ON decisions(spec, question_hash) WHERE label IS NOT NULL;
  CREATE TABLE IF NOT EXISTS calibrations (
    calibration_id TEXT PRIMARY KEY,
    spec TEXT NOT NULL,
    bands TEXT NOT NULL,
    report TEXT NOT NULL
  );
