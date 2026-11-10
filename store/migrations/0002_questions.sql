-- The question text by hash. Decisions keep only the hash, so without this a recurring ad-hoc question cannot be read
-- or saved as a spec.
CREATE TABLE IF NOT EXISTS questions (
    question_hash TEXT PRIMARY KEY,
    question TEXT NOT NULL
);
