use super::*;
use crate::conversations::tests::Fixture;
use std::fs;

fn configure(db: &Connection, base: &str) {
    let profiles = json!([{"id":"access-fixture","llm":"test-model","deployment":"local","apiBase":base,"apiKey":"","maxOutputTokens":2048,"contextWindow":32768}]);
    db.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles',json(?1),'$.activeModelProfileId','access-fixture')",[profiles.to_string()]).unwrap();
}
fn grant(f: &Fixture, item: &Conversation, ids: Vec<i64>) -> AccessSnapshot {
    let db = db::open(&f.path).unwrap();
    let old = snapshot(&db, item.project_id, &item.id).unwrap();
    set(
        &f.path,
        SetAccessInput {
            project_id: item.project_id,
            conversation_id: item.id.clone(),
            expected_revision: old.revision,
            agent_enabled: true,
            read_assets: true,
            read_scans: false,
            selected_skill_ids: ids,
            provider_disclosure_accepted: true,
            provider_identity: old.provider_identity,
            expected_skill_revision: Some(old.skill_revision),
        },
    )
    .unwrap()
}
fn add_skill(db: &Connection, name: &str, text: &str) -> i64 {
    db.execute("INSERT INTO agent_skills(name,description,instructions,enabled) VALUES(?1,'Synthetic guidance',?2,1)",params![name,text]).unwrap();
    db.last_insert_rowid()
}
#[test]
fn plain_chat_scope_validation_does_not_require_agent_mode() {
    let f = Fixture::new();
    let item = f.create();
    let db = db::open(&f.path).unwrap();
    snapshot(&db, 901, &item.id).unwrap();
    db.execute("INSERT INTO conversation_access(conversation_id,project_id,agent_enabled) VALUES(?1,901,0)",[&item.id]).unwrap();
    validate_scope_revision(&db, 901, &item.id, 1).unwrap();
    assert!(tool_specs(&db, &item).unwrap().is_empty());
    assert!(validate_scope_revision(&db, 901, &item.id, 2).is_err());
    assert!(validate_scope_revision(&db, 902, &item.id, 1).is_err());
}
#[test]
fn old_boolean_grants_are_inactive_and_recipient_is_sanitized() {
    let f = Fixture::new();
    let item = f.create();
    let db = db::open(&f.path).unwrap();
    configure(&db, "http://127.0.0.1:7/private-path?tenant=synthetic");
    snapshot(&db, 901, &item.id).unwrap();
    db.execute("INSERT INTO conversation_access(conversation_id,project_id,read_assets,provider_disclosure_accepted) VALUES(?1,901,1,1)",[&item.id]).unwrap();
    let scope = snapshot(&db, 901, &item.id).unwrap();
    assert!(!scope.provider_disclosure_accepted);
    assert_eq!(scope.provider_label.as_deref(), Some("http://127.0.0.1:7"));
    assert_eq!(scope.provider_identity.unwrap().len(), 64);
}
#[test]
fn endpoint_path_changes_require_new_recipient_acknowledgement() {
    let f = Fixture::new();
    let item = f.create();
    let db = db::open(&f.path).unwrap();
    configure(&db, "http://127.0.0.1:7/first");
    let initial = grant(&f, &item, vec![]);
    assert!(initial.provider_disclosure_accepted);
    configure(&db, "http://127.0.0.1:7/second");
    let next = snapshot(&db, 901, &item.id).unwrap();
    assert!(!next.provider_disclosure_accepted);
    assert_ne!(initial.provider_identity, next.provider_identity);
    assert!(set(
        &f.path,
        SetAccessInput {
            project_id: 901,
            conversation_id: item.id.clone(),
            expected_revision: next.revision,
            agent_enabled: true,
            read_assets: true,
            read_scans: false,
            selected_skill_ids: vec![],
            provider_disclosure_accepted: true,
            provider_identity: initial.provider_identity,
            expected_skill_revision: Some(initial.skill_revision)
        }
    )
    .is_err());
    assert!(grant(&f, &item, vec![]).provider_disclosure_accepted);
}
#[test]
fn changed_or_disabled_skill_invalidates_consent_and_inflight_scope() {
    let f = Fixture::new();
    let item = f.create();
    let db = db::open(&f.path).unwrap();
    configure(&db, "http://127.0.0.1:7/v1");
    let id = add_skill(&db, "read-guidance", "Read saved context only.");
    let initial = grant(&f, &item, vec![id]);
    let profile = crate::conversations::reply::resolve_profile(&db).unwrap();
    let scope = flight_scope(&db, 901, &item.id, initial.revision, &profile).unwrap();
    db.execute(
        "UPDATE agent_skills SET instructions='Edited guidance' WHERE id=?1",
        [id],
    )
    .unwrap();
    assert!(validate_flight_scope(&db, 901, &item.id, &scope).is_err());
    let changed = snapshot(&db, 901, &item.id).unwrap();
    assert!(changed.skills_require_review);
    assert!(!changed.provider_disclosure_accepted);
    assert_ne!(changed.skill_revision, initial.skill_revision);
    assert!(set(
        &f.path,
        SetAccessInput {
            project_id: 901,
            conversation_id: item.id.clone(),
            expected_revision: changed.revision,
            agent_enabled: true,
            read_assets: true,
            read_scans: false,
            selected_skill_ids: vec![id],
            provider_disclosure_accepted: true,
            provider_identity: changed.provider_identity,
            expected_skill_revision: Some(initial.skill_revision)
        }
    )
    .is_err());
    let renewed = grant(&f, &item, vec![id]);
    let scope = flight_scope(&db, 901, &item.id, renewed.revision, &profile).unwrap();
    db.execute("UPDATE agent_skills SET enabled=0 WHERE id=?1", [id])
        .unwrap();
    assert!(validate_flight_scope(&db, 901, &item.id, &scope).is_err());
    assert!(execute(&db, &item, "skills_read", &json!({"skillId":id})).is_err());
}
#[test]
fn thirty_six_selected_skills_are_listed_as_bounded_pages() {
    let f = Fixture::new();
    let item = f.create();
    let db = db::open(&f.path).unwrap();
    configure(&db, "http://127.0.0.1:7/v1");
    let ids = (0..36)
        .map(|i| add_skill(&db, &format!("synthetic-{i}"), "Reference only."))
        .collect::<Vec<_>>();
    grant(&f, &item, ids.clone());
    let mut found = vec![];
    let mut offset = 0;
    loop {
        let value = execute(&db, &item, "skills_list", &json!({"offset":offset})).unwrap();
        assert!(value.to_string().len() < 32_768);
        found.extend(
            value["skills"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["id"].as_i64().unwrap()),
        );
        let Some(next) = value["nextOffset"].as_u64() else {
            break;
        };
        offset = next;
    }
    assert_eq!(found, ids);
    assert!(execute(&db, &item, "skills_list", &json!({"limit":13})).is_err());
}
#[test]
fn skill_instruction_pages_preserve_utf8_and_reject_unselected_or_bad_offsets() {
    let f = Fixture::new();
    let item = f.create();
    let db = db::open(&f.path).unwrap();
    configure(&db, "http://127.0.0.1:7/v1");
    let text = "中a".repeat(5000);
    let id = add_skill(&db, "long-guidance", &text);
    grant(&f, &item, vec![id]);
    let mut rebuilt = String::new();
    let mut offset = 0;
    loop {
        let value = execute(
            &db,
            &item,
            "skills_read",
            &json!({"skillId":id,"offset":offset}),
        )
        .unwrap();
        rebuilt.push_str(value["content"].as_str().unwrap());
        assert!(value["content"].as_str().unwrap().len() <= 4096);
        let Some(next) = value["nextOffset"].as_u64() else {
            break;
        };
        offset = next;
    }
    assert_eq!(rebuilt, text);
    assert!(execute(&db, &item, "skills_read", &json!({"skillId":id,"offset":1})).is_err());
    assert!(execute(&db, &item, "skills_read", &json!({"skillId":id+1})).is_err());
    assert!(execute(
        &db,
        &item,
        "skills_read",
        &json!({"skillId":id,"limit":8192})
    )
    .is_err());
}
#[test]
fn revocation_remains_possible_after_recipient_changes() {
    let f = Fixture::new();
    let item = f.create();
    let db = db::open(&f.path).unwrap();
    configure(&db, "http://127.0.0.1:7/first");
    grant(&f, &item, vec![]);
    configure(&db, "http://127.0.0.1:7/second");
    let old = snapshot(&db, 901, &item.id).unwrap();
    let result = set(
        &f.path,
        SetAccessInput {
            project_id: 901,
            conversation_id: item.id.clone(),
            expected_revision: old.revision,
            agent_enabled: false,
            read_assets: false,
            read_scans: false,
            selected_skill_ids: vec![],
            provider_disclosure_accepted: false,
            provider_identity: old.provider_identity,
            expected_skill_revision: Some(old.skill_revision),
        },
    )
    .unwrap();
    assert!(!result.agent_enabled);
    assert!(!result.read_assets);
}
#[test]
fn legacy_access_schema_migrates_without_promoting_boolean_consent() {
    let f = Fixture::new();
    let item = f.create();
    let db = db::open(&f.path).unwrap();
    db.execute_batch("DROP TABLE conversation_access;CREATE TABLE conversation_access(conversation_id TEXT PRIMARY KEY,project_id INTEGER,revision INTEGER DEFAULT 1,agent_enabled INTEGER DEFAULT 1,read_assets INTEGER DEFAULT 0,read_scans INTEGER DEFAULT 0,selected_skill_ids TEXT DEFAULT '[]',provider_disclosure_accepted INTEGER DEFAULT 0);").unwrap();
    db.execute("INSERT INTO conversation_access(conversation_id,project_id,read_assets,provider_disclosure_accepted) VALUES(?1,901,1,1)",[&item.id]).unwrap();
    let value = snapshot(&db, 901, &item.id).unwrap();
    assert!(value.read_assets);
    assert!(!value.provider_disclosure_accepted);
}
#[cfg(unix)]
#[test]
fn directory_rejects_traversal_symlinks_and_credentials_after_output_boundary() {
    use std::os::unix::fs::symlink;
    let root = std::env::temp_dir().join(format!("oviraptor-access-safe-{}", Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    fs::write(root.join("safe.txt"), "safe guidance").unwrap();
    fs::write(
        root.join("later.txt"),
        format!(
            "{}\npassword = \"synthetic-access-value\"",
            "x".repeat(20_000)
        ),
    )
    .unwrap();
    symlink(root.join("safe.txt"), root.join("linked.txt")).unwrap();
    let granted = files::approve_root(&root).unwrap();
    assert_eq!(
        files::read(&granted, "safe.txt").unwrap()["content"],
        "safe guidance"
    );
    for path in ["../safe.txt", "linked.txt", ".env", "later.txt"] {
        assert!(files::read(&granted, path).is_err(), "{path}");
    }
    fs::remove_dir_all(root).unwrap();
}
#[cfg(unix)]
#[test]
fn directory_text_output_fits_complete_tool_envelope_even_with_json_escaping() {
    let root = std::env::temp_dir().join(format!("oviraptor-access-escaped-{}", Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    fs::write(root.join("escaped.txt"), "\t".repeat(8192)).unwrap();
    let granted = files::approve_root(&root).unwrap();
    let value = files::read(&granted, "escaped.txt").unwrap();
    assert!(value.to_string().len() < 32_768);
    assert_eq!(value["bytesRead"], 4096);
    assert_eq!(value["truncated"], true);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn absent_provider_never_blocks_plain_chat_or_creates_tool_consent() {
    let f = Fixture::new();
    let item = f.create();
    let db = db::open(&f.path).unwrap();
    db.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles',json('[]'))",[]).unwrap();
    let current = snapshot(&db, 901, &item.id).unwrap();
    assert!(current.provider_identity.is_none());
    assert!(current.provider_label.is_none());
    assert!(!current.provider_disclosure_accepted);
    let plain = set(
        &f.path,
        SetAccessInput {
            project_id: 901,
            conversation_id: item.id.clone(),
            expected_revision: current.revision,
            agent_enabled: false,
            read_assets: false,
            read_scans: false,
            selected_skill_ids: vec![],
            provider_disclosure_accepted: false,
            provider_identity: None,
            expected_skill_revision: Some(current.skill_revision),
        },
    )
    .unwrap();
    validate_scope_revision(&db, 901, &item.id, plain.revision).unwrap();
    assert!(tool_specs(&db, &item).unwrap().is_empty());
}

#[test]
fn disable_then_enable_at_same_timestamp_never_revives_prior_skill_consent() {
    let f = Fixture::new();
    let item = f.create();
    let db = db::open(&f.path).unwrap();
    configure(&db, "http://127.0.0.1:7/v1");
    let id = add_skill(&db, "same-time-enabled", "Unchanged guidance.");
    db.execute(
        "UPDATE agent_skills SET updated_at='same-instant' WHERE id=?1",
        [id],
    )
    .unwrap();
    let original = grant(&f, &item, vec![id]);
    db.execute(
        "UPDATE agent_skills SET enabled=0,updated_at='same-instant' WHERE id=?1",
        [id],
    )
    .unwrap();
    db.execute(
        "UPDATE agent_skills SET enabled=1,updated_at='same-instant' WHERE id=?1",
        [id],
    )
    .unwrap();
    let current = snapshot(&db, 901, &item.id).unwrap();
    assert!(current.skills_require_review);
    assert!(!current.provider_disclosure_accepted);
    assert_ne!(current.skill_revision, original.skill_revision);
    assert!(grant(&f, &item, vec![id]).provider_disclosure_accepted);
}
#[test]
fn editing_then_restoring_text_at_same_timestamp_never_revives_prior_skill_consent() {
    let f = Fixture::new();
    let item = f.create();
    let db = db::open(&f.path).unwrap();
    configure(&db, "http://127.0.0.1:7/v1");
    let id = add_skill(&db, "same-time-edited", "Original guidance.");
    db.execute(
        "UPDATE agent_skills SET updated_at='same-instant' WHERE id=?1",
        [id],
    )
    .unwrap();
    let original = grant(&f, &item, vec![id]);
    db.execute("UPDATE agent_skills SET instructions='Temporary change',updated_at='same-instant' WHERE id=?1",[id]).unwrap();
    db.execute("UPDATE agent_skills SET instructions='Original guidance.',updated_at='same-instant' WHERE id=?1",[id]).unwrap();
    let current = snapshot(&db, 901, &item.id).unwrap();
    assert!(current.skills_require_review);
    assert!(!current.provider_disclosure_accepted);
    assert_ne!(current.skill_revision, original.skill_revision);
}
#[test]
fn deleting_and_recreating_same_skill_id_keeps_revocation_tombstone() {
    let f = Fixture::new();
    let item = f.create();
    let db = db::open(&f.path).unwrap();
    configure(&db, "http://127.0.0.1:7/v1");
    let id = add_skill(&db, "same-id", "Original guidance.");
    let original = grant(&f, &item, vec![id]);
    db.execute("DELETE FROM agent_skills WHERE id=?1", [id])
        .unwrap();
    db.execute("INSERT INTO agent_skills(id,name,description,instructions,enabled) VALUES(?1,'same-id','Synthetic guidance','Original guidance.',1)",[id]).unwrap();
    let current = snapshot(&db, 901, &item.id).unwrap();
    assert!(current.skills_require_review);
    assert!(!current.provider_disclosure_accepted);
    assert_ne!(current.skill_revision, original.skill_revision);
}
