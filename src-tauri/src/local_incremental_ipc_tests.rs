//! Actual Tauri argument decoding, async handlers and SQLite integration.
//! MockRuntime substitutes the native window/event loop only. No domain, DB,
//! handler or notification installer is extracted or replaced by a stub.
use crate::{commands, db, AppState};
use rusqlite::params;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use tauri::{
    test::{mock_builder, mock_context, noop_assets, MockRuntime},
    App, Manager, WebviewWindow,
};

struct Fixture {
    _app: App<MockRuntime>,
    window: WebviewWindow<MockRuntime>,
    root: PathBuf,
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("oviraptor-local-ipc-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = db::initialize(&root).unwrap();
        let c = db::open(&path).unwrap();
        c.execute(
            "INSERT INTO projects(id,name) VALUES(9001,'Synthetic IPC'),(9002,'Control IPC')",
            [],
        )
        .unwrap();
        let body = "<p>Identical complete synthetic response</p>";
        let hash = format!("{:x}", Sha256::digest(body.as_bytes()));
        let content_type = "text/html; charset=utf-8";
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(
                json!(["http-full-body-v1", 200, body.len(), hash, content_type])
                    .to_string()
                    .as_bytes()
            )
        );
        let proof = json!({"probe_outcome":"web_alive","probe_status_code":"200","probe_body_complete":"true","probe_body_truncated":"false","probe_body_bytes":body.len().to_string(),"probe_body_limit":"4096","probe_content_type":content_type,"body_sha256":hash,"probe_response_fingerprint":fingerprint,"probe_content_encoding":"","probe_content_range":"","probe_error":""});
        for id in 1..=4 {
            c.execute("INSERT INTO assets(id,asset_key,host,link,title,status_code,probe_outcome,extra_json) VALUES(?1,?2,?2,?3,'Synthetic shared page','200','web_alive',?4)",params![id,format!("ipc-{id}.example.test"),format!("https://ipc-{id}.example.test/path"),proof.to_string()]).unwrap();
            c.execute(
                "INSERT INTO project_assets(project_id,asset_id) VALUES(?1,?2)",
                params![if id == 4 { 9002 } else { 9001 }, id],
            )
            .unwrap();
        }
        c.execute("UPDATE project_assets SET decision='confirmed',note='manual retention' WHERE project_id=9001 AND asset_id=3",[]).unwrap();
        drop(c);
        let app = mock_builder()
            .manage(AppState {
                db_path: path.clone(),
                app_data_dir: root.clone(),
                legacy_icon_dirs: Vec::new(),
                export_dir: root.join("exports"),
                cancellations: Arc::new(Mutex::new(HashMap::new())),
                active_jobs: Arc::new(AtomicUsize::new(0)),
                worker_service: Default::default(),
            })
            .invoke_handler(tauri::generate_handler![
                commands::page_isolation::preview_page_rule,
                commands::page_isolation::apply_page_rule,
                commands::page_isolation::list_page_rules,
                commands::page_isolation::disable_page_rule,
                commands::asset_rule_catalog::get_asset_rule_catalog,
                commands::assets::list_assets,
                commands::assets::update_decision,
                commands::assets::soft_delete_assets,
                commands::asset_export::export_assets,
                crate::jobs::cancel_job,
            ])
            .build(mock_context(noop_assets()))
            .unwrap();
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        Self {
            _app: app,
            window,
            root,
            path,
        }
    }
    fn invoke(&self, cmd: &str, body: Value) -> Result<Value, Value> {
        tauri::test::get_ipc_response(
            &self.window,
            tauri::webview::InvokeRequest {
                cmd: cmd.into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: if cfg!(any(windows, target_os = "android")) {
                    "http://tauri.localhost"
                } else {
                    "tauri://localhost"
                }
                .parse()
                .unwrap(),
                body: tauri::ipc::InvokeBody::Json(body),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.into(),
            },
        )
        .map(|response| response.deserialize::<Value>().unwrap())
    }
    fn rule(&self) -> Value {
        json!({"projectId":9001,"sourceAssetId":1,"kind":"title_contains","value":"Synthetic shared"})
    }
    fn preview(&self) -> Value {
        self.invoke("preview_page_rule", json!({"input":self.rule()}))
            .unwrap()
    }
    fn state(&self, project: i64, asset: i64) -> (String, String, bool) {
        db::open(&self.path).unwrap().query_row("SELECT decision,note,is_deleted FROM project_assets WHERE project_id=?1 AND asset_id=?2",params![project,asset],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn registered_page_handlers_preserve_manual_state_and_restore_exact_project_rows() {
    let f = Fixture::new();
    let preview = f.preview();
    assert_eq!(
        (
            preview["matched"].as_u64(),
            preview["eligible"].as_u64(),
            preview["protected"].as_u64()
        ),
        (Some(3), Some(2), Some(1))
    );
    assert_eq!(preview["rule"]["projectId"], 9001);
    let applied = f
        .invoke(
            "apply_page_rule",
            json!({"input":{"rule":f.rule(),"previewToken":preview["previewToken"]}}),
        )
        .unwrap();
    assert_eq!(
        (applied["isolated"].as_u64(), applied["protected"].as_u64()),
        (Some(2), Some(1))
    );
    let backup = PathBuf::from(applied["backupPath"].as_str().unwrap());
    assert!(backup.starts_with(&f.root));
    let backup_json: Value = serde_json::from_slice(&fs::read(&backup).unwrap()).unwrap();
    assert_eq!(backup_json["schemaVersion"], 1);
    assert_eq!(backup_json["operation"], "apply");
    assert!(f.state(9001, 1).2);
    assert_eq!(
        f.state(9001, 3),
        ("confirmed".into(), "manual retention".into(), false)
    );
    assert!(!f.state(9002, 4).2);
    let list = f
        .invoke("list_page_rules", json!({"projectId":9001}))
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], applied["ruleId"]);
    assert_eq!(list[0]["enabled"], true);
    let catalog = f.invoke("get_asset_rule_catalog", json!({})).unwrap();
    assert_eq!(catalog["projectRules"], list);
    assert!(catalog["legacyRules"].is_array());
    assert!(f
        .invoke(
            "disable_page_rule",
            json!({"projectId":9002,"ruleId":applied["ruleId"]})
        )
        .is_err());
    assert!(f.state(9001, 1).2);
    let disabled = f
        .invoke(
            "disable_page_rule",
            json!({"projectId":9001,"ruleId":applied["ruleId"]}),
        )
        .unwrap();
    assert_eq!(disabled["restored"], 2);
    assert_eq!(disabled["retained"], 0);
    assert!(PathBuf::from(disabled["backupPath"].as_str().unwrap()).is_file());
    for asset in [1, 2] {
        assert_eq!(f.state(9001, asset), ("pending".into(), "".into(), false));
    }
    assert_eq!(
        f.invoke("list_page_rules", json!({"projectId":9001}))
            .unwrap()[0]["enabled"],
        false
    );
}

#[test]
fn registered_page_apply_rejects_stale_human_decisions_and_wrong_project_inputs() {
    let f = Fixture::new();
    let preview = f.preview();
    f.invoke("update_decision",json!({"input":{"projectId":9001,"assetIds":[2],"decision":"confirmed","note":"human after preview"}})).unwrap();
    let error = f
        .invoke(
            "apply_page_rule",
            json!({"input":{"rule":f.rule(),"previewToken":preview["previewToken"]}}),
        )
        .unwrap_err();
    assert!(error.to_string().contains("重新预览"));
    assert!(!f.state(9001, 1).2);
    assert_eq!(
        f.state(9001, 2),
        ("confirmed".into(), "human after preview".into(), false)
    );
    assert_eq!(
        f.invoke("list_page_rules", json!({"projectId":9001}))
            .unwrap(),
        json!([])
    );
    let mut wrong = f.rule();
    wrong["projectId"] = json!(9002);
    assert!(f
        .invoke("preview_page_rule", json!({"input":wrong}))
        .is_err());
    let fresh = f.preview();
    let applied = f
        .invoke(
            "apply_page_rule",
            json!({"input":{"rule":f.rule(),"previewToken":fresh["previewToken"]}}),
        )
        .unwrap();
    assert_eq!(applied["isolated"], 1);
    // The old restore command remains compatible and overrides enabled rules.
    f.invoke(
        "soft_delete_assets",
        json!({"projectId":9001,"assetIds":[1],"deleted":false}),
    )
    .unwrap();
    assert_eq!(f.state(9001, 1), ("pending".into(), "".into(), false));
    let disabled = f
        .invoke(
            "disable_page_rule",
            json!({"projectId":9001,"ruleId":applied["ruleId"]}),
        )
        .unwrap();
    assert_eq!(disabled["restored"], 0);
    assert!(!f.state(9001, 1).2);
}

#[test]
fn registered_asset_list_keeps_legacy_defaults_and_group_member_wire_contracts() {
    let f = Fixture::new();
    let raw = f
        .invoke("list_assets", json!({"query":{"projectId":9001}}))
        .unwrap();
    assert_eq!(
        (raw["total"].as_u64(), raw["memberTotal"].as_u64()),
        (Some(3), Some(3))
    );
    assert_eq!(raw["items"].as_array().unwrap().len(), 3);
    let folded=f.invoke("list_assets",json!({"query":{"projectId":9001,"collapseIdenticalResponses":true,"page":1,"pageSize":10}})).unwrap();
    assert_eq!(
        (folded["total"].as_u64(), folded["memberTotal"].as_u64()),
        (Some(2), Some(3))
    );
    let representative = folded["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["extra"]["response_display_group_count"] == 2)
        .unwrap();
    let fingerprint = &representative["extra"]["probe_response_fingerprint"];
    let expanded = f
        .invoke(
            "list_assets",
            json!({"query":{"projectId":9001,"responseFingerprint":fingerprint}}),
        )
        .unwrap();
    assert_eq!(expanded["total"], 3);
    assert!(expanded["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["projectId"] == 9001));
    let anchored = f
        .invoke(
            "list_assets",
            json!({"query": {
                "projectId":9001,"responseFingerprint":fingerprint,
                "responseRepresentativeAssetId":representative["id"]
            }}),
        )
        .unwrap();
    assert_eq!(
        (anchored["total"].as_u64(), anchored["memberTotal"].as_u64()),
        (Some(2), Some(2))
    );
    assert!(anchored["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["id"] != 3));
    let foreign_anchor = f
        .invoke(
            "list_assets",
            json!({"query":{
                "projectId":9001,"responseFingerprint":fingerprint,"responseRepresentativeAssetId":4
            }}),
        )
        .unwrap();
    assert_eq!(foreign_anchor["total"], 0);
    assert!(f
        .invoke(
            "list_assets",
            json!({"query":{
                "projectId":9001,"responseRepresentativeAssetId":representative["id"]
            }})
        )
        .is_err());
    assert!(f
        .invoke(
            "list_assets",
            json!({"query":{"responseFingerprint":fingerprint}})
        )
        .is_err());
    assert!(f
        .invoke("list_page_rules", json!({"project_id":9001}))
        .is_err());
    assert!(f
        .invoke(
            "apply_page_rule",
            json!({"input":{"rule":f.rule(),"preview_token":"wrong casing"}})
        )
        .is_err());
}

#[test]
fn registered_restore_keeps_a_human_reconfirmed_isolation_decision_even_with_same_note() {
    let f = Fixture::new();
    let preview = f.preview();
    let applied = f
        .invoke(
            "apply_page_rule",
            json!({"input":{"rule":f.rule(),"previewToken":preview["previewToken"]}}),
        )
        .unwrap();
    let (_, applied_note, deleted) = f.state(9001, 1);
    assert!(deleted);
    f.invoke("update_decision",json!({"input":{"projectId":9001,"assetIds":[1],"decision":"rejected","note":applied_note}})).unwrap();
    f.invoke(
        "soft_delete_assets",
        json!({"projectId":9001,"assetIds":[1],"deleted":false}),
    )
    .unwrap();
    assert_eq!(f.state(9001, 1), ("rejected".into(), applied_note, false));
    f.invoke(
        "disable_page_rule",
        json!({"projectId":9001,"ruleId":applied["ruleId"]}),
    )
    .unwrap();
    assert_eq!(f.state(9001, 1).0, "rejected");
    assert_eq!(f.state(9001, 2), ("pending".into(), "".into(), false));
}

#[test]
fn registered_cancel_never_overwrites_terminal_runs_while_worker_cleanup_is_pending() {
    let f = Fixture::new();
    let c = db::open(&f.path).unwrap();
    for (id, status) in [(501, "completed"), (502, "failed"), (503, "cancelled")] {
        c.execute("INSERT INTO runs(id,project_id,name,pipeline,status,error,finished_at) VALUES(?1,9001,'Synthetic terminal','collect',?2,'original terminal detail','2026-01-01 00:00:00')",params![id,status]).unwrap();
        let flag = Arc::new(AtomicBool::new(false));
        f._app
            .state::<AppState>()
            .cancellations
            .lock()
            .unwrap()
            .insert(id, flag);
        assert!(f.invoke("cancel_job", json!({"runId":id})).is_err());
        let actual: (String, String, String) = c
            .query_row(
                "SELECT status,error,finished_at FROM runs WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            actual,
            (
                status.into(),
                "original terminal detail".into(),
                "2026-01-01 00:00:00".into()
            )
        );
    }
}

#[test]
fn registered_cancel_persists_active_cancellation_and_reports_database_failure() {
    let f = Fixture::new();
    let c = db::open(&f.path).unwrap();
    c.execute("INSERT INTO runs(id,project_id,name,pipeline,status) VALUES(504,9001,'Synthetic active','collect','running'),(505,9001,'Synthetic blocked','collect','running')",[]).unwrap();
    let active = Arc::new(AtomicBool::new(false));
    let blocked = Arc::new(AtomicBool::new(false));
    {
        let state = f._app.state::<AppState>();
        let mut registry = state.cancellations.lock().unwrap();
        registry.insert(504, active.clone());
        registry.insert(505, blocked.clone());
    }
    assert!(f.invoke("cancel_job", json!({"runId":504})).is_ok());
    assert!(active.load(Ordering::Relaxed));
    let status: String = c
        .query_row("SELECT status FROM runs WHERE id=504", [], |r| r.get(0))
        .unwrap();
    assert_eq!(status, "cancel_requested");
    c.execute_batch("CREATE TRIGGER reject_synthetic_cancel BEFORE UPDATE OF status ON runs WHEN OLD.id=505 AND NEW.status='cancel_requested' BEGIN SELECT RAISE(ABORT,'synthetic cancellation write failure'); END;").unwrap();
    let error = f.invoke("cancel_job", json!({"runId":505})).unwrap_err();
    assert!(error
        .to_string()
        .contains("synthetic cancellation write failure"));
    assert!(blocked.load(Ordering::Relaxed));
    let status: String = c
        .query_row("SELECT status FROM runs WHERE id=505", [], |r| r.get(0))
        .unwrap();
    assert_eq!(status, "running");
}

#[test]
fn registered_exports_keep_all_raw_members_and_validate_exact_group_inputs_before_writing() {
    let f = Fixture::new();
    let raw = f
        .invoke(
            "export_assets",
            json!({"request":{
                "query":{"projectId":9001,"collapseIdenticalResponses":true},
                "fields":["assetKey","link"],"chineseHeaders":false
            }}),
        )
        .unwrap();
    assert_eq!(raw["rows"], 3);
    let path = PathBuf::from(raw["path"].as_str().unwrap());
    assert!(path.starts_with(f.root.join("exports")));
    let bytes = fs::read(path).unwrap();
    assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
    let mut csv = csv::Reader::from_reader(bytes.as_slice());
    assert_eq!(
        csv.headers().unwrap(),
        &csv::StringRecord::from(vec!["assetKey", "link"])
    );
    let records = csv.records().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(records.len(), 3);
    assert_eq!(
        records
            .iter()
            .map(|r| r[1].to_string())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        3
    );

    // A separate disposable output directory also keeps this test independent
    // of the export filename's clock resolution.
    let group = Fixture::new();
    let assets = group
        .invoke("list_assets", json!({"query":{"projectId":9001}}))
        .unwrap();
    let fingerprint = &assets["items"][0]["extra"]["probe_response_fingerprint"];
    let exported=group.invoke("export_assets",json!({"request":{
        "query":{"projectId":9001,"responseFingerprint":fingerprint,"responseRepresentativeAssetId":1},
        "fields":["assetKey","decision"],"chineseHeaders":false
    }})).unwrap();
    assert_eq!(exported["rows"], 2);
    let path = PathBuf::from(exported["path"].as_str().unwrap());
    let mut csv = csv::Reader::from_path(path).unwrap();
    let rows = csv.records().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| &r[1] == "pending"));
    let before = fs::read_dir(group.root.join("exports")).unwrap().count();
    for query in [
        json!({"responseFingerprint":fingerprint,"responseRepresentativeAssetId":1}),
        json!({"projectId":9001,"responseRepresentativeAssetId":1}),
        json!({"projectId":9001,"responseFingerprint":fingerprint,"responseRepresentativeAssetId":0}),
    ] {
        assert!(group
            .invoke(
                "export_assets",
                json!({"request":{"query":query,"fields":["assetKey"]}})
            )
            .is_err());
    }
    assert_eq!(
        fs::read_dir(group.root.join("exports")).unwrap().count(),
        before
    );
}
