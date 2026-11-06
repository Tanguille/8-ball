use std::{fs, os::unix::fs::OpenOptionsExt};

use sqlx::SqlSafeStr;

use crate::{
    backend::Probs,
    verdict::{Bands, Report},
};

#[derive(Clone)]
pub struct Store {
    pool: sqlx::SqlitePool,
}

pub struct NewDecision {
    pub spec: Option<String>,
    pub question_hash: String,
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
        // Lock before inspecting the schema so concurrent legacy opens cannot race the rebuild.
        let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
        let schema: Option<String> = sqlx::query_scalar(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='decisions'",
        )
        .fetch_optional(&mut *transaction)
        .await?;
        if schema.is_some_and(|sql| !sql.to_uppercase().contains("AUTOINCREMENT")) {
            sqlx::raw_sql("ALTER TABLE decisions RENAME TO decisions_previous;")
                .execute(&mut *transaction)
                .await?;
            sqlx::raw_sql(SCHEMA).execute(&mut *transaction).await?;
            sqlx::raw_sql(
                "INSERT INTO decisions SELECT * FROM decisions_previous; DROP TABLE decisions_previous;",
            )
                .execute(&mut *transaction)
                .await?;
            // Index names belonged to the renamed table until it was dropped.
            sqlx::raw_sql(SCHEMA).execute(&mut *transaction).await?;
        }
        let migrator =
            sqlx::migrate::Migrator::with_migrations(vec![sqlx::migrate::Migration::new(
                1,
                "storage".into(),
                sqlx::migrate::MigrationType::Simple,
                SCHEMA.into_sql_str(),
                false,
            )]);
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

    pub(crate) async fn metadata(
        &self,
        id: i64,
    ) -> Result<Option<(Option<String>, String)>, StoreError> {
        Ok(
            sqlx::query_as("SELECT spec,question_hash FROM decisions WHERE id=?1")
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
        spec: &str,
        question_hash: &str,
    ) -> Result<Vec<Labelled>, StoreError> {
        let rows: Vec<(i64, String, String, String)> = sqlx::query_as(
            "SELECT id,state,state_hash,label FROM decisions WHERE spec=?1 AND question_hash=?2 AND label IS NOT NULL ORDER BY id",
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

    pub async fn put_calibration(
        &self,
        calibration_id: &str,
        spec: &str,
        bands: &Bands,
        report: &Report,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO calibrations (calibration_id,spec,bands,report) VALUES (?1,?2,?3,?4) ON CONFLICT(calibration_id) DO UPDATE SET bands=excluded.bands,report=excluded.report",
        )
            .bind(calibration_id)
            .bind(spec)
            .bind(serde_json::to_string(bands)?)
            .bind(serde_json::to_string(report)?)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn calibration(
        &self,
        calibration_id: &str,
    ) -> Result<Option<(Bands, Report)>, StoreError> {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT bands,report FROM calibrations WHERE calibration_id=?1")
                .bind(calibration_id)
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

        usize::try_from(result.rows_affected()).map_err(|e| StoreError(e.to_string()))
    }
}

const SCHEMA: &str = include_str!("../migrations/0001_storage.sql");
