use std::{fs, os::unix::fs::OpenOptionsExt};

use sqlx::SqlSafeStr;

use domain::{
    Probs,
    verdict::{self, Bands, Report},
};

#[derive(Clone)]
pub struct Store {
    pool: sqlx::SqlitePool,
}

pub struct NewDecision {
    pub spec: Option<String>,
    pub question_hash: String,
    /// The question as JSON, without the caller's id, which is not part of the hash.
    pub question: String,
    pub calibration_id: String,
    pub state_hash: String,
    pub state: String,
    pub answer: Probs,
    pub verdict: String,
    pub endpoint: String,
}
pub struct Labelled {
    pub id: i64,
    pub state: String,
    pub state_hash: String,
    pub label: String,
}
/// Stored decisions grouped by spec and question version.
pub struct QuestionRow {
    pub spec: Option<String>,
    pub question_hash: String,
    pub question: Option<String>,
    pub decisions: u64,
    pub unsure: u64,
    pub labelled: u64,
    pub last_at: String,
}
pub struct DecisionRow {
    pub id: i64,
    pub at: String,
    pub verdict: String,
    pub answer: Probs,
    pub state: String,
    pub label: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct StoreError(pub String);
impl From<sqlx::Error> for StoreError {
    fn from(e: sqlx::Error) -> Self {
        Self(e.to_string())
    }
}
impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        Self(e.to_string())
    }
}

impl Store {
    /// Creates the file privately before SQLite creates its WAL and shared-memory files.
    pub async fn open(path: &std::path::Path) -> Result<Self, StoreError> {
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory)?;
        }
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)?;
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(5));
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        // Lock so concurrent first opens cannot both run the migrator.
        let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
        let migration = |version, description: &str, sql: &'static str| {
            sqlx::migrate::Migration::new(
                version,
                description.to_owned().into(),
                sqlx::migrate::MigrationType::Simple,
                sql.into_sql_str(),
                false,
            )
        };
        let migrator = sqlx::migrate::Migrator::with_migrations(vec![
            migration(1, "storage", SCHEMA),
            migration(2, "questions", QUESTIONS),
        ]);
        migrator
            .run(&mut *transaction)
            .await
            .map_err(|e| StoreError(e.to_string()))?;
        transaction.commit().await?;

        Ok(Self { pool })
    }

    pub async fn insert_many(&self, decisions: Vec<NewDecision>) -> Result<Vec<i64>, StoreError> {
        let mut transaction = self.pool.begin().await?;
        let mut ids = Vec::with_capacity(decisions.len());
        for decision in decisions {
            sqlx::query("INSERT OR IGNORE INTO questions (question_hash,question) VALUES (?1,?2)")
                .bind(&decision.question_hash)
                .bind(decision.question)
                .execute(&mut *transaction)
                .await?;
            let answer = serde_json::to_string(&decision.answer)?;
            let result = sqlx::query(
                "INSERT INTO decisions (spec,question_hash,calibration_id,state_hash,state,answer,verdict,endpoint) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            )
            .bind(decision.spec)
            .bind(decision.question_hash)
            .bind(decision.calibration_id)
            .bind(decision.state_hash)
            .bind(decision.state)
            .bind(answer)
            .bind(decision.verdict)
            .bind(decision.endpoint)
            .execute(&mut *transaction)
            .await?;
            ids.push(result.last_insert_rowid());
        }
        transaction.commit().await?;

        Ok(ids)
    }

    pub async fn cached(
        &self,
        calibration_id: &str,
        state_hash: &str,
        spec: Option<&str>,
    ) -> Result<Option<(i64, Probs)>, StoreError> {
        let row: Option<(i64, String)> = sqlx::query_as(
            "SELECT id,answer FROM decisions WHERE calibration_id=?1 AND state_hash=?2 AND spec IS ?3 ORDER BY id DESC LIMIT 1",
        )
        .bind(calibration_id)
        .bind(state_hash)
        .bind(spec)
        .fetch_optional(&self.pool)
        .await?;

        row.map(|(id, answer)| Ok((id, serde_json::from_str(&answer)?)))
            .transpose()
    }

    /// Returns `(spec, question_hash, endpoint)`.
    pub async fn metadata(
        &self,
        id: i64,
    ) -> Result<Option<(Option<String>, String, String)>, StoreError> {
        Ok(
            sqlx::query_as("SELECT spec,question_hash,endpoint FROM decisions WHERE id=?1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// Returns false when no row has this id.
    pub async fn set_label(&self, id: i64, label: &str) -> Result<bool, StoreError> {
        let result = sqlx::query("UPDATE decisions SET label=?1 WHERE id=?2")
            .bind(label)
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn labelled(
        &self,
        spec: Option<&str>,
        question_hash: &str,
    ) -> Result<Vec<Labelled>, StoreError> {
        let rows: Vec<(i64, String, String, String)> = sqlx::query_as(
            "SELECT id,state,state_hash,label FROM decisions WHERE spec IS ?1 AND question_hash=?2 AND label IS NOT NULL ORDER BY id",
        )
        .bind(spec)
        .bind(question_hash)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, state, state_hash, label)| Labelled {
                id,
                state,
                state_hash,
                label,
            })
            .collect())
    }

    /// The newest labeled decision for this question and state, under any model: a label is the real answer.
    pub async fn remembered(
        &self,
        spec: Option<&str>,
        question_hash: &str,
        state_hash: &str,
    ) -> Result<Option<(i64, Probs, String)>, StoreError> {
        let row: Option<(i64, String, String)> = sqlx::query_as(
            "SELECT id,answer,label FROM decisions WHERE spec IS ?1 AND question_hash=?2 AND state_hash=?3 AND label IS NOT NULL ORDER BY id DESC LIMIT 1",
        )
        .bind(spec)
        .bind(question_hash)
        .bind(state_hash)
        .fetch_optional(&self.pool)
        .await?;

        row.map(|(id, answer, label)| Ok((id, serde_json::from_str(&answer)?, label)))
            .transpose()
    }

    pub async fn labelled_count(
        &self,
        spec: Option<&str>,
        question_hash: &str,
    ) -> Result<usize, StoreError> {
        let count: u64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM decisions WHERE spec IS ?1 AND question_hash=?2 AND label IS NOT NULL",
        )
        .bind(spec)
        .bind(question_hash)
        .fetch_one(&self.pool)
        .await?;

        usize::try_from(count).map_err(|e| StoreError(e.to_string()))
    }

    /// Stores bands under `calibration_key`, so an ad-hoc question never shares a spec's calibration.
    pub async fn put_calibration(
        &self,
        calibration_id: &str,
        spec: Option<&str>,
        bands: &Bands,
        report: &Report,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO calibrations (calibration_id,spec,bands,report) VALUES (?1,?2,?3,?4) ON CONFLICT(calibration_id) DO UPDATE SET bands=excluded.bands,report=excluded.report",
        )
        .bind(verdict::calibration_key(calibration_id, spec))
        // The column is NOT NULL: an ad-hoc calibration stores an empty spec.
        .bind(spec.unwrap_or(""))
        .bind(serde_json::to_string(bands)?)
        .bind(serde_json::to_string(report)?)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn calibration(
        &self,
        calibration_id: &str,
        spec: Option<&str>,
    ) -> Result<Option<(Bands, Report)>, StoreError> {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT bands,report FROM calibrations WHERE calibration_id=?1")
                .bind(verdict::calibration_key(calibration_id, spec))
                .fetch_optional(&self.pool)
                .await?;

        row.map(|(bands, report)| {
            Ok((
                serde_json::from_str(&bands)?,
                serde_json::from_str(&report)?,
            ))
        })
        .transpose()
    }

    pub async fn prune(&self, retention_days: u32) -> Result<usize, StoreError> {
        let result =
            sqlx::query("DELETE FROM decisions WHERE label IS NULL AND ts < datetime('now',?1)")
                .bind(format!("-{retention_days} days"))
                .execute(&self.pool)
                .await?;
        sqlx::query(
            "DELETE FROM questions WHERE question_hash NOT IN (SELECT question_hash FROM decisions)",
        )
        .execute(&self.pool)
        .await?;

        usize::try_from(result.rows_affected()).map_err(|e| StoreError(e.to_string()))
    }

    pub async fn questions(&self, limit: u32) -> Result<Vec<QuestionRow>, StoreError> {
        // spec, question_hash, question, decisions, unsure, labelled, last_at
        type Row = (
            Option<String>,
            String,
            Option<String>,
            u64,
            u64,
            u64,
            String,
        );
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT d.spec,d.question_hash,q.question,COUNT(*),SUM(d.verdict='unsure'),COUNT(d.label),MAX(d.ts) \
             FROM decisions d LEFT JOIN questions q ON q.question_hash=d.question_hash \
             GROUP BY d.spec,d.question_hash ORDER BY COUNT(*) DESC,MAX(d.ts) DESC LIMIT ?1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(spec, question_hash, question, decisions, unsure, labelled, last_at)| {
                    QuestionRow {
                        spec,
                        question_hash,
                        question,
                        decisions,
                        unsure,
                        labelled,
                        last_at,
                    }
                },
            )
            .collect())
    }

    pub async fn question(&self, question_hash: &str) -> Result<Option<String>, StoreError> {
        Ok(
            sqlx::query_scalar("SELECT question FROM questions WHERE question_hash=?1")
                .bind(question_hash)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// Newest first. `spec` None selects ad-hoc decisions.
    pub async fn decisions(
        &self,
        spec: Option<&str>,
        question_hash: &str,
        unlabelled: bool,
        limit: u32,
    ) -> Result<Vec<DecisionRow>, StoreError> {
        let rows: Vec<(i64, String, String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT id,ts,verdict,answer,state,label FROM decisions \
             WHERE spec IS ?1 AND question_hash=?2 AND (?3=0 OR label IS NULL) ORDER BY id DESC LIMIT ?4",
        )
        .bind(spec)
        .bind(question_hash)
        .bind(unlabelled)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|(id, at, verdict, answer, state, label)| {
                Ok(DecisionRow {
                    id,
                    at,
                    verdict,
                    answer: serde_json::from_str(&answer)?,
                    state,
                    label,
                })
            })
            .collect()
    }
}

const SCHEMA: &str = include_str!("../migrations/0001_storage.sql");
const QUESTIONS: &str = include_str!("../migrations/0002_questions.sql");
