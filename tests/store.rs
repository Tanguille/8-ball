use std::os::unix::fs::PermissionsExt;

use sqlx::{Connection, SqliteConnection};

use eightball::{
    store::{NewDecision, Store},
    verdict::{Bands, Report},
};

async fn fixture(path: &std::path::Path) -> SqliteConnection {
    SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true),
    )
    .await
    .unwrap()
}

fn temp() -> tempfile::TempDir {
    tempfile::Builder::new().prefix("8ball-").tempdir().unwrap()
}
fn decision(state_hash: &str) -> NewDecision {
    NewDecision {
        spec: Some("greeting".into()),
        question_hash: "question".into(),
        calibration_id: "model-question".into(),
        state_hash: state_hash.into(),
        state: "hello".into(),
        answer: vec![0.9, 0.1],
        verdict: "yes".into(),
        endpoint: "d3".into(),
    }
}
const fn report() -> Report {
    Report {
        examples: 40,
        decided: 40,
        correct: 40,
        precision: 1.0,
        coverage: 1.0,
        target: 0.95,
        in_sample: true,
        warnings: vec![],
    }
}
#[tokio::test]
async fn file_is_private_and_migrations_are_idempotent() {
    let directory = temp();
    let path = directory.path().join("db.sqlite");
    let _first = Store::open(&path).await.unwrap();
    let _second = Store::open(&path).await.unwrap();
    for name in ["db.sqlite", "db.sqlite-wal", "db.sqlite-shm"] {
        assert_eq!(
            std::fs::metadata(directory.path().join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
#[tokio::test]
async fn insert_and_cache_lookup() {
    let directory = temp();
    let store = Store::open(&directory.path().join("db.sqlite"))
        .await
        .unwrap();
    let id = store.insert(decision("first")).await.unwrap();
    let (cached_id, answer) = store
        .cached("model-question", "first", Some("greeting"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cached_id, id);
    assert!((answer[0] - 0.9).abs() < 1e-9);
    assert!(
        store
            .cached("model-question", "other", Some("greeting"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .cached("other-model", "first", Some("greeting"))
            .await
            .unwrap()
            .is_none()
    );
}
#[tokio::test]
async fn labels_can_be_replaced_and_are_scoped() {
    let directory = temp();
    let store = Store::open(&directory.path().join("db.sqlite"))
        .await
        .unwrap();
    let id = store.insert(decision("first")).await.unwrap();
    assert_eq!(
        store.set_label(id, "yes").await.unwrap(),
        Some((Some("greeting".into()), "question".into()))
    );
    let rows = store.labelled("greeting", "question").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].label, "yes");
    assert_eq!(rows[0].id, id);
    store.set_label(id, "no").await.unwrap();
    assert_eq!(
        store.labelled("greeting", "question").await.unwrap()[0].label,
        "no"
    );
    assert!(
        store
            .labelled("greeting", "edited-question")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(store.set_label(99_999, "yes").await.unwrap().is_none());
}
#[tokio::test]
async fn pruning_preserves_labelled_rows() {
    let directory = temp();
    let path = directory.path().join("db.sqlite");
    let store = Store::open(&path).await.unwrap();
    store.insert(decision("first")).await.unwrap();
    let labelled = store.insert(decision("second")).await.unwrap();
    store.set_label(labelled, "yes").await.unwrap();
    sqlx::query("UPDATE decisions SET ts=datetime('now','-1 day')")
        .execute(&mut fixture(&path).await)
        .await
        .unwrap();
    assert_eq!(store.prune(0).await.unwrap(), 1);
    assert!(
        store
            .cached("model-question", "first", Some("greeting"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .cached("model-question", "second", Some("greeting"))
            .await
            .unwrap()
            .is_some()
    );
}
#[tokio::test]
async fn calibration_upsert_keeps_latest_bands() {
    let directory = temp();
    let path = directory.path().join("db.sqlite");
    let store = Store::open(&path).await.unwrap();
    store
        .put_calibration(
            "cid",
            "greeting",
            &Bands::Noul {
                yes_at: 0.9,
                no_at: 0.1,
            },
            &report(),
        )
        .await
        .unwrap();
    store
        .put_calibration(
            "cid",
            "greeting",
            &Bands::Noul {
                yes_at: 0.8,
                no_at: 0.2,
            },
            &report(),
        )
        .await
        .unwrap();
    let (bands, report) = store.calibration("cid").await.unwrap().unwrap();
    let Bands::Noul { yes_at, no_at } = bands else {
        panic!("noul bands")
    };
    assert!((yes_at - 0.8).abs() < 1e-9);
    assert!((no_at - 0.2).abs() < 1e-9);
    assert_eq!(report.examples, 40);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM calibrations")
            .fetch_one(&mut fixture(&path).await)
            .await
            .unwrap(),
        1
    );
}
#[tokio::test]
async fn batch_insert_returns_each_row_id() {
    let directory = temp();
    let store = Store::open(&directory.path().join("db.sqlite"))
        .await
        .unwrap();
    let ids = store
        .insert_many(vec![decision("one"), decision("two")])
        .await
        .unwrap();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    assert_eq!(
        store
            .cached("model-question", "two", Some("greeting"))
            .await
            .unwrap()
            .unwrap()
            .0,
        ids[1]
    );
}

#[tokio::test]
async fn pruned_ids_never_label_replacement_rows() {
    let directory = temp();
    let path = directory.path().join("db.sqlite");
    let store = Store::open(&path).await.unwrap();
    let old = store.insert(decision("old")).await.unwrap();
    sqlx::query("UPDATE decisions SET ts=datetime('now','-1 day')")
        .execute(&mut fixture(&path).await)
        .await
        .unwrap();
    assert_eq!(store.prune(0).await.unwrap(), 1);
    drop(store);
    let reopened = Store::open(&path).await.unwrap();
    let new = reopened.insert(decision("replacement")).await.unwrap();
    assert!(new > old, "pruned id {old} was reused as {new}");
    assert!(reopened.set_label(old, "yes").await.unwrap().is_none());
    assert!(
        reopened
            .labelled("greeting", "question")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn existing_database_migrates_without_losing_labels_or_ids() {
    let directory = temp();
    let path = directory.path().join("db.sqlite");
    let id = 42;
    let mut connection = fixture(&path).await;
    sqlx::raw_sql(LEGACY_SCHEMA)
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::raw_sql("INSERT INTO decisions VALUES (42,CURRENT_TIMESTAMP,'greeting','question','model-question','saved','hello','[0.9,0.1]','yes','d3','yes')")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("INSERT INTO calibrations VALUES ('cid','greeting',?1,?2)")
        .bind(
            serde_json::to_string(&Bands::Noul {
                yes_at: 0.8,
                no_at: 0.2,
            })
            .unwrap(),
        )
        .bind(serde_json::to_string(&report()).unwrap())
        .execute(&mut connection)
        .await
        .unwrap();
    drop(connection);
    let store = Store::open(&path).await.unwrap();
    let (_, saved_report) = store.calibration("cid").await.unwrap().unwrap();
    assert_eq!(saved_report.examples, 40);
    let rows = store.labelled("greeting", "question").await.unwrap();
    assert_eq!(rows[0].id, id);
    assert_eq!(rows[0].label, "yes");
    let old = store.insert(decision("old")).await.unwrap();
    sqlx::query("UPDATE decisions SET ts=datetime('now','-1 day')")
        .execute(&mut fixture(&path).await)
        .await
        .unwrap();
    store.prune(0).await.unwrap();
    let new = store.insert(decision("new")).await.unwrap();
    assert!(new > old);
    assert!(store.set_label(old, "no").await.unwrap().is_none());
}

const LEGACY_SCHEMA: &str  =  "
CREATE TABLE decisions (
 id INTEGER PRIMARY KEY, ts TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
 spec TEXT, question_hash TEXT NOT NULL, calibration_id TEXT NOT NULL,
 state_hash TEXT NOT NULL, state TEXT NOT NULL, answer TEXT NOT NULL,
 verdict TEXT NOT NULL, endpoint TEXT NOT NULL, label TEXT);
CREATE INDEX decisions_cache ON decisions(calibration_id,state_hash);
CREATE INDEX decisions_labelled ON decisions(spec,question_hash) WHERE label IS NOT NULL;
CREATE TABLE calibrations (calibration_id TEXT PRIMARY KEY,spec TEXT NOT NULL,bands TEXT NOT NULL,report TEXT NOT NULL);
";

#[tokio::test]
async fn existing_autoincrement_preserves_deleted_highwater_and_calibration() {
    let directory = temp();
    let path = directory.path().join("db.sqlite");
    let mut connection = fixture(&path).await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(LEGACY_SCHEMA.replace(
        "id INTEGER PRIMARY KEY,",
        "id INTEGER PRIMARY KEY AUTOINCREMENT,",
    )))
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::raw_sql("INSERT INTO decisions VALUES (100,CURRENT_TIMESTAMP,NULL,'question','model-question','old','hello','[0.9,0.1]','yes','d3',NULL); DELETE FROM decisions;")
        .execute(&mut connection)
        .await
        .unwrap();
    let bands = Bands::Noul {
        yes_at: 0.8,
        no_at: 0.2,
    };
    sqlx::query("INSERT INTO calibrations VALUES ('cid','greeting',?1,?2)")
        .bind(serde_json::to_string(&bands).unwrap())
        .bind(serde_json::to_string(&report()).unwrap())
        .execute(&mut connection)
        .await
        .unwrap();
    drop(connection);
    for _ in 0..2 {
        let store = Store::open(&path).await.unwrap();
        assert!(store.insert(decision("new")).await.unwrap() > 100);
        let (saved, report) = store.calibration("cid").await.unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(saved).unwrap(),
            serde_json::to_value(bands).unwrap()
        );
        assert_eq!(report.examples, 40);
    }
}

#[tokio::test]
async fn batch_failure_rolls_back_all_rows() {
    let directory = temp();
    let path = directory.path().join("db.sqlite");
    let store = Store::open(&path).await.unwrap();
    sqlx::raw_sql("CREATE TRIGGER reject_bad BEFORE INSERT ON decisions WHEN NEW.state_hash='bad' BEGIN SELECT RAISE(ABORT,'rejected'); END;")
        .execute(&mut fixture(&path).await)
        .await
        .unwrap();
    assert!(
        store
            .insert_many(vec![decision("good"), decision("bad")])
            .await
            .is_err()
    );
    assert!(
        store
            .cached("model-question", "good", Some("greeting"))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn cloned_store_serializes_concurrent_writes() {
    let directory = temp();
    let store = Store::open(&directory.path().join("db.sqlite"))
        .await
        .unwrap();
    let tasks = (0..20).map(|i| {
        let store = store.clone();
        tokio::spawn(async move { store.insert(decision(&i.to_string())).await.unwrap() })
    });
    let mut ids = futures::future::join_all(tasks)
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 20);
}

#[tokio::test]
async fn cache_preserves_null_scope_and_latest_row_and_rejects_bad_json() {
    let directory = temp();
    let path = directory.path().join("db.sqlite");
    let store = Store::open(&path).await.unwrap();
    store.insert(decision("same")).await.unwrap();
    let mut anonymous = decision("same");
    anonymous.spec = None;
    let id = store.insert(anonymous).await.unwrap();
    assert_eq!(
        store
            .cached("model-question", "same", None)
            .await
            .unwrap()
            .unwrap()
            .0,
        id
    );
    let newest = store.insert(decision("same")).await.unwrap();
    assert_eq!(
        store
            .cached("model-question", "same", Some("greeting"))
            .await
            .unwrap()
            .unwrap()
            .0,
        newest
    );
    sqlx::query("UPDATE decisions SET answer='invalid' WHERE id=?1")
        .bind(newest)
        .execute(&mut fixture(&path).await)
        .await
        .unwrap();
    assert!(
        store
            .cached("model-question", "same", Some("greeting"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn concurrent_legacy_opens_preserve_rows_and_indexes() {
    let directory = temp();
    let path = directory.path().join("db.sqlite");
    let mut connection = fixture(&path).await;
    sqlx::raw_sql(LEGACY_SCHEMA)
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::raw_sql("INSERT INTO decisions VALUES (42,CURRENT_TIMESTAMP,'greeting','question','model-question','saved','hello','[0.9,0.1]','yes','d3','yes')")
        .execute(&mut connection)
        .await
        .unwrap();
    drop(connection);
    let (first_store, second_store) = tokio::join!(Store::open(&path), Store::open(&path));
    let first_store = first_store.unwrap();
    let second_store = second_store.unwrap();
    assert_ne!(
        first_store.insert(decision("a")).await.unwrap(),
        second_store.insert(decision("b")).await.unwrap()
    );
    assert_eq!(
        first_store.labelled("greeting", "question").await.unwrap()[0].id,
        42
    );
    assert_eq!(
        second_store.labelled("greeting", "question").await.unwrap()[0].label,
        "yes"
    );
    let mut connection = fixture(&path).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_schema WHERE type='index' AND name IN ('decisions_cache','decisions_labelled')")
            .fetch_one(&mut connection)
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn failed_legacy_upgrade_rolls_back_schema_and_data() {
    let directory = temp();
    let path = directory.path().join("db.sqlite");
    let mut connection = fixture(&path).await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(
        LEGACY_SCHEMA.replace("state TEXT NOT NULL", "state TEXT"),
    ))
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::raw_sql("INSERT INTO decisions VALUES (42,CURRENT_TIMESTAMP,'greeting','question','model-question','saved',NULL,'[0.9,0.1]','yes','d3','yes'); INSERT INTO calibrations VALUES ('cid','greeting','{}','{}');")
        .execute(&mut connection)
        .await
        .unwrap();
    assert!(Store::open(&path).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM decisions WHERE id=42 AND state IS NULL"
        )
        .fetch_one(&mut connection)
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM calibrations")
            .fetch_one(&mut connection)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_schema WHERE type='index' AND name IN ('decisions_cache','decisions_labelled')")
            .fetch_one(&mut connection)
            .await
            .unwrap(),
        2
    );
    let schema: String = sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE name='decisions'")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert!(!schema.contains("AUTOINCREMENT"));
    sqlx::query("UPDATE decisions SET state='hello'")
        .execute(&mut connection)
        .await
        .unwrap();
    drop(connection);
    let store = Store::open(&path).await.unwrap();
    assert_eq!(
        store.labelled("greeting", "question").await.unwrap()[0].id,
        42
    );
}
