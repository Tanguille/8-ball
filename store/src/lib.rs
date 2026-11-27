use std::{fs, os::unix::fs::OpenOptionsExt};

use sqlx::types::Json;

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
#[derive(sqlx::FromRow)]
pub struct Labelled {
    pub id: i64,
    pub state: String,
    pub state_hash: String,
    pub label: String,
}
/// Stored decisions grouped by spec and question version.
#[derive(sqlx::FromRow)]
pub struct QuestionRow {
    pub spec: Option<String>,
    pub question_hash: String,
    pub question: Option<String>,
    pub decisions: u64,
    pub unsure: u64,
    pub labelled: u64,
    pub last_at: String,
}
#[derive(sqlx::FromRow)]
pub struct DecisionRow {
    pub id: i64,
    pub at: String,
    pub verdict: String,
    #[sqlx(json)]
    pub answer: Probs,
    pub state: String,
    pub label: Option<String>,
    pub endpoint: String,
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
        sqlx::migrate!()
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
            let result = sqlx::query(
                "INSERT INTO decisions (spec,question_hash,calibration_id,state_hash,state,answer,verdict,endpoint) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            )
            .bind(decision.spec)
            .bind(decision.question_hash)
            .bind(decision.calibration_id)
            .bind(decision.state_hash)
            .bind(decision.state)
            .bind(Json(decision.answer))
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
        let row: Option<(i64, Json<Probs>)> = sqlx::query_as(
            "SELECT id,answer FROM decisions WHERE calibration_id=?1 AND state_hash=?2 AND spec IS ?3 ORDER BY id DESC LIMIT 1",
        )
        .bind(calibration_id)
        .bind(state_hash)
        .bind(spec)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(id, Json(answer))| (id, answer)))
    }

    /// Returns `(spec, question_hash, state_hash)`.
    pub async fn metadata(
        &self,
        id: i64,
    ) -> Result<Option<(Option<String>, String, String)>, StoreError> {
        Ok(
            sqlx::query_as("SELECT spec,question_hash,state_hash FROM decisions WHERE id=?1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// The endpoints that answered this question about this state, first answer first.
    pub async fn endpoints(
        &self,
        spec: Option<&str>,
        question_hash: &str,
        state_hash: &str,
    ) -> Result<Vec<String>, StoreError> {
        Ok(sqlx::query_scalar(
            "SELECT endpoint FROM decisions WHERE spec IS ?1 AND question_hash=?2 AND state_hash=?3 GROUP BY endpoint ORDER BY MIN(id)",
        )
        .bind(spec)
        .bind(question_hash)
        .bind(state_hash)
        .fetch_all(&self.pool)
        .await?)
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
        Ok(sqlx::query_as(
            "SELECT id,state,state_hash,label FROM decisions WHERE spec IS ?1 AND question_hash=?2 AND label IS NOT NULL ORDER BY id",
        )
        .bind(spec)
        .bind(question_hash)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The newest labeled decision for this question and state, under any model: a label is the real answer.
    pub async fn remembered(
        &self,
        spec: Option<&str>,
        question_hash: &str,
        state_hash: &str,
    ) -> Result<Option<(i64, Probs, String)>, StoreError> {
        let row: Option<(i64, Json<Probs>, String)> = sqlx::query_as(
            "SELECT id,answer,label FROM decisions WHERE spec IS ?1 AND question_hash=?2 AND state_hash=?3 AND label IS NOT NULL ORDER BY id DESC LIMIT 1",
        )
        .bind(spec)
        .bind(question_hash)
        .bind(state_hash)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(id, Json(answer), label)| (id, answer, label)))
    }

    /// Labelled states of a question that have an answer cached under `calibration_id`: the examples a refit of
    /// that calibration can use.
    pub async fn labelled_states(
        &self,
        spec: Option<&str>,
        question_hash: &str,
        calibration_id: &str,
    ) -> Result<usize, StoreError> {
        let count: u64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT state_hash) FROM decisions AS labelled WHERE spec IS ?1 AND question_hash=?2 AND label IS NOT NULL \
             AND EXISTS (SELECT 1 FROM decisions WHERE calibration_id=?3 AND state_hash=labelled.state_hash AND spec IS ?1)",
        )
        .bind(spec)
        .bind(question_hash)
        .bind(calibration_id)
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
        .bind(Json(bands))
        .bind(Json(report))
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn calibration(
        &self,
        calibration_id: &str,
        spec: Option<&str>,
    ) -> Result<Option<(Bands, Report)>, StoreError> {
        let row: Option<(Json<Bands>, Json<Report>)> =
            sqlx::query_as("SELECT bands,report FROM calibrations WHERE calibration_id=?1")
                .bind(verdict::calibration_key(calibration_id, spec))
                .fetch_optional(&self.pool)
                .await?;

        Ok(row.map(|(Json(bands), Json(report))| (bands, report)))
    }

    /// Deletes old unlabelled rows, but keeps every row of a labelled state: each endpoint that answered it refits
    /// from its own row.
    pub async fn prune(&self, retention_days: u32) -> Result<usize, StoreError> {
        let result = sqlx::query(
            "DELETE FROM decisions WHERE label IS NULL AND ts < datetime('now',?1) \
             AND NOT EXISTS (SELECT 1 FROM decisions l \
               WHERE l.spec IS decisions.spec AND l.question_hash=decisions.question_hash \
                 AND l.state_hash=decisions.state_hash AND l.label IS NOT NULL)",
        )
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
        Ok(sqlx::query_as(
            "SELECT d.spec,d.question_hash,q.question,COUNT(*) AS decisions,SUM(d.verdict='unsure') AS unsure,\
             COUNT(d.label) AS labelled,MAX(d.ts) AS last_at \
             FROM decisions d LEFT JOIN questions q ON q.question_hash=d.question_hash \
             GROUP BY d.spec,d.question_hash ORDER BY COUNT(*) DESC,MAX(d.ts) DESC LIMIT ?1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn question(&self, question_hash: &str) -> Result<Option<String>, StoreError> {
        Ok(
            sqlx::query_scalar("SELECT question FROM questions WHERE question_hash=?1")
                .bind(question_hash)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// Newest first. `spec` None selects ad-hoc decisions. `unlabelled` lists the newest row of each state that has no
    /// label on any row, so a case answered by two endpoints is reviewed once.
    pub async fn decisions(
        &self,
        spec: Option<&str>,
        question_hash: &str,
        unlabelled: bool,
        limit: u32,
    ) -> Result<Vec<DecisionRow>, StoreError> {
        Ok(sqlx::query_as(
            "SELECT id,ts AS at,verdict,answer,state,label,endpoint FROM decisions d \
             WHERE spec IS ?1 AND question_hash=?2 \
               AND (?3=0 OR ( \
                 id = (SELECT MAX(id) FROM decisions m \
                       WHERE m.spec IS d.spec AND m.question_hash=d.question_hash AND m.state_hash=d.state_hash) \
                 AND NOT EXISTS (SELECT 1 FROM decisions l \
                       WHERE l.spec IS d.spec AND l.question_hash=d.question_hash AND l.state_hash=d.state_hash \
                         AND l.label IS NOT NULL))) \
             ORDER BY id DESC LIMIT ?4",
        )
        .bind(spec)
        .bind(question_hash)
        .bind(unlabelled)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }
}
