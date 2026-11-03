use eightball::{
    store::{NewDecision, Store},
    verdict::{Bands, Report},
};
use std::os::unix::fs::PermissionsExt;

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
#[test]
fn file_is_private_and_migrations_are_idempotent() {
    let dir = temp();
    let path = dir.path().join("db.sqlite");
    let _first = Store::open(&path).unwrap();
    let _second = Store::open(&path).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
#[tokio::test]
async fn insert_and_cache_lookup() {
    let dir = temp();
    let store = Store::open(&dir.path().join("db.sqlite")).unwrap();
    let id = store.insert(decision("first")).await.unwrap();
    let (cached_id, answer) = store
        .cached("model-question", "first")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cached_id, id);
    assert!((answer[0] - 0.9).abs() < 1e-9);
    assert!(
        store
            .cached("model-question", "other")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .cached("other-model", "first")
            .await
            .unwrap()
            .is_none()
    );
}
#[tokio::test]
async fn labels_can_be_replaced_and_are_scoped() {
    let dir = temp();
    let store = Store::open(&dir.path().join("db.sqlite")).unwrap();
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
    let dir = temp();
    let path = dir.path().join("db.sqlite");
    let store = Store::open(&path).unwrap();
    store.insert(decision("first")).await.unwrap();
    let labelled = store.insert(decision("second")).await.unwrap();
    store.set_label(labelled, "yes").await.unwrap();
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("UPDATE decisions SET ts = datetime('now','-1 day')", [])
        .unwrap();
    assert_eq!(store.prune(0).await.unwrap(), 1);
    assert!(
        store
            .cached("model-question", "first")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .cached("model-question", "second")
            .await
            .unwrap()
            .is_some()
    );
}
#[tokio::test]
async fn calibration_upsert_keeps_latest_bands() {
    let dir = temp();
    let path = dir.path().join("db.sqlite");
    let store = Store::open(&path).unwrap();
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
        rusqlite::Connection::open(&path)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM calibrations", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        1
    );
}
#[tokio::test]
async fn batch_insert_returns_each_row_id() {
    let dir = temp();
    let store = Store::open(&dir.path().join("db.sqlite")).unwrap();
    let ids = store
        .insert_many(vec![decision("one"), decision("two")])
        .await
        .unwrap();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    assert_eq!(
        store
            .cached("model-question", "two")
            .await
            .unwrap()
            .unwrap()
            .0,
        ids[1]
    );
}
