use std::os::unix::fs::PermissionsExt;

use sqlx::{Connection, SqliteConnection};
use tempfile::TempDir;

use domain::verdict::{Bands, Report};
use store::{NewDecision, Store};

async fn fixture(path: &std::path::Path) -> SqliteConnection {
    SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true),
    )
    .await
    .unwrap()
}

fn temp() -> TempDir {
    tempfile::Builder::new().prefix("8ball-").tempdir().unwrap()
}
async fn opened() -> (TempDir, std::path::PathBuf, Store) {
    let directory = temp();
    let path = directory.path().join("db.sqlite");
    let store = Store::open(&path).await.unwrap();

    (directory, path, store)
}
async fn insert(store: &Store, decision: NewDecision) -> i64 {
    store.insert_many(vec![decision]).await.unwrap()[0]
}
async fn cached(
    store: &Store,
    state_hash: &str,
    spec: Option<&str>,
) -> Option<(i64, domain::Probs)> {
    store
        .cached("model-question", state_hash, spec)
        .await
        .unwrap()
}
fn decision(state_hash: &str) -> NewDecision {
    NewDecision {
        spec: Some("greeting".into()),
        question_hash: "question".into(),
        question: r#"{"instructions":"Greeting?","type":"noul"}"#.into(),
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
    let (_directory, _, store) = opened().await;
    let id = insert(&store, decision("first")).await;
    let (cached_id, answer) = cached(&store, "first", Some("greeting")).await.unwrap();
    assert_eq!(cached_id, id);
    assert!((answer[0] - 0.9).abs() < 1e-9);
    assert!(cached(&store, "other", Some("greeting")).await.is_none());
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
    let (_directory, _, store) = opened().await;
    let id = insert(&store, decision("first")).await;
    assert!(store.set_label(id, "yes").await.unwrap());
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
    assert!(!store.set_label(99_999, "yes").await.unwrap());
}
#[tokio::test]
async fn pruning_preserves_labelled_rows() {
    let (_directory, path, store) = opened().await;
    insert(&store, decision("first")).await;
    let labelled = insert(&store, decision("second")).await;
    store.set_label(labelled, "yes").await.unwrap();
    sqlx::query("UPDATE decisions SET ts=datetime('now','-1 day')")
        .execute(&mut fixture(&path).await)
        .await
        .unwrap();
    assert_eq!(store.prune(0).await.unwrap(), 1);
    assert!(cached(&store, "first", Some("greeting")).await.is_none());
    assert!(cached(&store, "second", Some("greeting")).await.is_some());
}
#[tokio::test]
async fn calibration_upsert_keeps_latest_bands() {
    let (_directory, path, store) = opened().await;
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
    let (_directory, _, store) = opened().await;
    let ids = store
        .insert_many(vec![decision("one"), decision("two")])
        .await
        .unwrap();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    assert_eq!(
        cached(&store, "two", Some("greeting")).await.unwrap().0,
        ids[1]
    );
}

#[tokio::test]
async fn pruned_ids_never_label_replacement_rows() {
    let (_directory, path, store) = opened().await;
    let old = insert(&store, decision("old")).await;
    sqlx::query("UPDATE decisions SET ts=datetime('now','-1 day')")
        .execute(&mut fixture(&path).await)
        .await
        .unwrap();
    assert_eq!(store.prune(0).await.unwrap(), 1);
    drop(store);
    let reopened = Store::open(&path).await.unwrap();
    let new = insert(&reopened, decision("replacement")).await;
    assert!(new > old, "pruned id {old} was reused as {new}");
    assert!(!reopened.set_label(old, "yes").await.unwrap());
    assert!(
        reopened
            .labelled("greeting", "question")
            .await
            .unwrap()
            .is_empty()
    );
}

const LEGACY_SCHEMA: &str  =  "
CREATE TABLE decisions (
 id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
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
    sqlx::raw_sql(LEGACY_SCHEMA)
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
        assert!(insert(&store, decision("new")).await > 100);
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
    let (_directory, path, store) = opened().await;
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
    assert!(cached(&store, "good", Some("greeting")).await.is_none());
}

#[tokio::test]
async fn cloned_store_serializes_concurrent_writes() {
    let (_directory, _, store) = opened().await;
    let tasks = (0..20).map(|i| {
        let store = store.clone();
        tokio::spawn(async move { insert(&store, decision(&i.to_string())).await })
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
    let (_directory, path, store) = opened().await;
    insert(&store, decision("same")).await;
    let mut anonymous = decision("same");
    anonymous.spec = None;
    let id = insert(&store, anonymous).await;
    assert_eq!(cached(&store, "same", None).await.unwrap().0, id);
    let newest = insert(&store, decision("same")).await;
    assert_eq!(
        cached(&store, "same", Some("greeting")).await.unwrap().0,
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
