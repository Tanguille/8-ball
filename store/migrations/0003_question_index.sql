-- The decision list of one question searches this index. The per-question counts and the orphan sweep in prune read
-- only this index, not the large state and answer columns.
CREATE INDEX IF NOT EXISTS decisions_question ON decisions(spec, question_hash, id, verdict, ts, label);
