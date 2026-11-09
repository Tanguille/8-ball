-- The question text by hash. Decisions keep only the hash, so without this a recurring ad-hoc question cannot be read
-- or saved as a spec.
CREATE TABLE IF NOT EXISTS questions (
    question_hash TEXT PRIMARY KEY,
    question TEXT NOT NULL
);
-- The decision list of one question searches this index. The per-question counts and the orphan sweep in prune read
-- only this index, not the large state and answer columns.
CREATE INDEX IF NOT EXISTS decisions_question ON decisions(spec, question_hash, id, verdict, ts, label);
