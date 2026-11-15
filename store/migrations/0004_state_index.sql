-- The unlabelled decision list finds the newest row and any label of each state through this index.
CREATE INDEX IF NOT EXISTS decisions_state ON decisions(spec, question_hash, state_hash, id, label);
