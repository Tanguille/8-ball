use crate::{
    backend::Probs,
    verdict::{Bands, Report},
};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
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
impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self(e.to_string())
    }
}
impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}

impl Store {
    /// Creates the file with mode 0600 first (so -wal/-shm inherit it), then opens it,
    /// sets pragmas, runs migrations.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        use std::os::unix::fs::OpenOptionsExt;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)?;
        let mut conn = Connection::open(path)?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get::<_, String>(0))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        let tx = conn.transaction()?;
        let schema: Option<String> = tx
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type='table' AND name='decisions'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if schema.is_some_and(|sql| !sql.to_uppercase().contains("AUTOINCREMENT")) {
            // Preserve saved ids and labels when upgrading the original v1 table.
            tx.execute_batch("ALTER TABLE decisions RENAME TO decisions_previous;")?;
            tx.execute_batch(MIGRATIONS)?;
            tx.execute_batch(
                "INSERT INTO decisions SELECT * FROM decisions_previous;
                DROP TABLE decisions_previous;",
            )?;
        }
        tx.execute_batch(MIGRATIONS)?;
        tx.commit()?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    async fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> Result<T, StoreError> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().expect("store mutex poisoned");
            f(&mut guard)
        })
        .await
        .map_err(|e| StoreError(e.to_string()))?
        .map_err(StoreError::from)
    }

    pub async fn insert(&self, d: NewDecision) -> Result<i64, StoreError> {
        Ok(self
            .insert_many(vec![d])
            .await?
            .into_iter()
            .next()
            .expect("one inserted decision"))
    }

    /// Store all decisions in one transaction.
    pub async fn insert_many(&self, ds: Vec<NewDecision>) -> Result<Vec<i64>, StoreError> {
        self.call(move |conn| {
            let tx=conn.transaction()?;let mut ids=Vec::with_capacity(ds.len());
            for d in ds {
                let answer=serde_json::to_string(&d.answer).map_err(|e|rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                tx.execute("INSERT INTO decisions (spec,question_hash,calibration_id,state_hash,state,answer,verdict,endpoint) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",params![d.spec,d.question_hash,d.calibration_id,d.state_hash,d.state,answer,d.verdict,d.endpoint])?;
                ids.push(tx.last_insert_rowid());
            }
            tx.commit()?;Ok(ids)
        }).await
    }

    pub async fn cached(
        &self,
        calibration_id: &str,
        state_hash: &str,
        spec: Option<&str>,
    ) -> Result<Option<(i64, Probs)>, StoreError> {
        let cid = calibration_id.to_owned();
        let state_hash = state_hash.to_owned();
        let spec = spec.map(str::to_owned);
        self.call(move |conn|conn.query_row("SELECT id,answer FROM decisions WHERE calibration_id=?1 AND state_hash=?2 AND spec IS ?3 ORDER BY id DESC LIMIT 1",params![cid,state_hash,spec],|row|Ok((row.get(0)?,json_column(row,1)?))).optional()).await
    }

    pub(crate) async fn metadata(
        &self,
        id: i64,
    ) -> Result<Option<(Option<String>, String)>, StoreError> {
        self.call(move |conn| {
            conn.query_row(
                "SELECT spec,question_hash FROM decisions WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
        })
        .await
    }

    /// Returns (spec, `question_hash`) of the row, None if no row.
    pub async fn set_label(
        &self,
        id: i64,
        label: &str,
    ) -> Result<Option<(Option<String>, String)>, StoreError> {
        let label = label.to_owned();
        self.call(move |conn| {
            let tx = conn.transaction()?;
            let metadata = tx
                .query_row(
                    "SELECT spec,question_hash FROM decisions WHERE id=?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if metadata.is_some() {
                tx.execute(
                    "UPDATE decisions SET label=?1 WHERE id=?2",
                    params![label, id],
                )?;
            }
            tx.commit()?;
            Ok(metadata)
        })
        .await
    }

    pub async fn labelled(
        &self,
        spec: &str,
        question_hash: &str,
    ) -> Result<Vec<Labelled>, StoreError> {
        let spec = spec.to_owned();
        let question_hash = question_hash.to_owned();
        self.call(move |conn| {
            let mut stmt=conn.prepare("SELECT id,state,state_hash,label FROM decisions WHERE spec=?1 AND question_hash=?2 AND label IS NOT NULL ORDER BY id")?;
            stmt.query_map(params![spec,question_hash],|row|Ok(Labelled {id:row.get(0)?,state:row.get(1)?,state_hash:row.get(2)?,label:row.get(3)?}))?.collect()
        }).await
    }

    pub async fn put_calibration(
        &self,
        calibration_id: &str,
        spec: &str,
        bands: &Bands,
        report: &Report,
    ) -> Result<(), StoreError> {
        let cid = calibration_id.to_owned();
        let spec = spec.to_owned();
        let bands = serde_json::to_string(bands).map_err(|e| StoreError(e.to_string()))?;
        let report = serde_json::to_string(report).map_err(|e| StoreError(e.to_string()))?;
        self.call(move |conn| {
            conn.execute("INSERT INTO calibrations (calibration_id,spec,bands,report) VALUES (?1,?2,?3,?4) ON CONFLICT(calibration_id) DO UPDATE SET bands=excluded.bands,report=excluded.report",params![cid,spec,bands,report])?;Ok(())
        }).await
    }

    pub async fn calibration(
        &self,
        calibration_id: &str,
    ) -> Result<Option<(Bands, Report)>, StoreError> {
        let cid = calibration_id.to_owned();
        self.call(move |conn| {
            conn.query_row(
                "SELECT bands,report FROM calibrations WHERE calibration_id=?1",
                [cid],
                |row| Ok((json_column(row, 0)?, json_column(row, 1)?)),
            )
            .optional()
        })
        .await
    }

    pub async fn prune(&self, retention_days: u32) -> Result<usize, StoreError> {
        self.call(move |conn| {
            conn.execute(
                "DELETE FROM decisions WHERE label IS NULL AND ts < datetime('now',?1)",
                [format!("-{retention_days} days")],
            )
        })
        .await
    }
}

fn json_column<T: serde::de::DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<T> {
    let text: String = row.get(index)?;
    serde_json::from_str(&text).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(e))
    })
}

const MIGRATIONS: &str = r"
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
  ";
