use super::*;
use rusqlite::{params, Connection};

fn settings(model: &str, base: &str) -> JsonValue {
    serde_json::json!({
        "activeModelProfileId":"selected",
        "modelProfiles":[{"id":"selected","llm":format!("openai/{model}"),
            "deployment":"local","apiBase":base,"localApiKey":"synthetic-local-key"}],
        "modelApiKey":"synthetic-cloud-key-never-used"
    })
}

fn connection() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE config_profiles(id INTEGER PRIMARY KEY,is_default INTEGER NOT NULL,
         settings_json TEXT NOT NULL,updated_at TEXT NOT NULL);",
        )
        .unwrap();
    connection
}

#[test]
fn default_profile_keeps_original_priority_and_id_order_without_writes() {
    let connection = connection();
    for (id, is_default, model, updated) in [
        (1, 0, "not-default", "2099-01-01"),
        (10, 1, "original-first", "2000-01-01"),
        (20, 1, "newer-list-row", "2099-01-01"),
    ] {
        connection
            .execute(
                "INSERT INTO config_profiles VALUES(?1,?2,?3,?4)",
                params![
                    id,
                    is_default,
                    settings(model, "http://127.0.0.1:8123/v1").to_string(),
                    updated
                ],
            )
            .unwrap();
    }
    let changes = connection.total_changes();
    let profile = gateway_profile_on(&connection, None).unwrap();
    assert_eq!(profile.model, "original-first");
    assert_eq!(
        profile.endpoint,
        "http://127.0.0.1:8123/v1/chat/completions"
    );
    assert_eq!(profile.bearer(), Some("synthetic-local-key"));
    assert!(profile.proxy.is_none());
    assert!(connection.is_autocommit());
    assert_eq!(connection.total_changes(), changes);
}

#[test]
fn profile_resolution_reads_the_callers_uncommitted_snapshot_without_committing() {
    let connection = connection();
    let original = settings("before", "http://127.0.0.1:8123/v1").to_string();
    connection
        .execute(
            "INSERT INTO config_profiles VALUES(1,1,?1,'2000-01-01')",
            [&original],
        )
        .unwrap();
    let snapshot = connection.unchecked_transaction().unwrap();
    snapshot
        .execute(
            "UPDATE config_profiles SET settings_json=?1 WHERE id=1",
            [settings("snapshot-model", "http://127.0.0.1:8456/custom/path").to_string()],
        )
        .unwrap();
    let changes = snapshot.total_changes();
    let profile = gateway_profile_on(&snapshot, None).unwrap();
    assert_eq!(profile.model, "snapshot-model");
    assert_eq!(
        profile.endpoint,
        "http://127.0.0.1:8456/custom/path/chat/completions"
    );
    assert!(!snapshot.is_autocommit());
    assert_eq!(snapshot.total_changes(), changes);
    snapshot.rollback().unwrap();
    assert_eq!(default_settings(&connection).to_string(), original);
    assert_eq!(
        gateway_profile_on(&connection, None).unwrap().model,
        "before"
    );
}
