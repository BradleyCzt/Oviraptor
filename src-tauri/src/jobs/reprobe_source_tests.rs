use super::*;

struct Fixture {
    root: PathBuf,
    path: PathBuf,
    connection: rusqlite::Connection,
    refined: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("oviraptor-reprobe-input-{}", Uuid::new_v4()));
        let path = db::initialize(&root).unwrap();
        let connection = db::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO projects(id,name) VALUES(9001,'Synthetic reprobe')",
                [],
            )
            .unwrap();
        for id in 1..=2 {
            connection.execute("INSERT INTO assets(id,asset_key,host,link,title) VALUES(?1,?2,?2,?3,'multiline\noriginal title')",
                params![id, format!("reprobe-{id}.example.test"), format!("https://reprobe-{id}.example.test/")]).unwrap();
            connection.execute("INSERT INTO project_assets(project_id,asset_id,decision,note) VALUES(9001,?1,'confirmed','human decision')", [id]).unwrap();
        }
        let refined = root.join("jobs/original/refined");
        Self {
            root,
            path,
            connection,
            refined,
        }
    }
    fn write(&self) -> Result<i64, String> {
        write_reprobe_sources(&self.path, 9001, &self.refined)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn reprobe_publication_failure_has_no_primary_marker_and_retry_keeps_all_assets() {
    let f = Fixture::new();
    fs::create_dir_all(f.refined.join("P3_name_candidates.csv")).unwrap();
    assert!(f.write().is_err());
    assert!(!f.refined.join("P1_active_strong.csv").exists());
    fs::remove_dir(f.refined.join("P3_name_candidates.csv")).unwrap();
    assert_eq!(f.write().unwrap(), 2);
    let (_, count) = validate_reprobe_source(&f.refined.join("P1_active_strong.csv")).unwrap();
    assert_eq!(count, 2);
    assert_eq!(f.connection.query_row("SELECT COUNT(*) FROM project_assets WHERE decision='confirmed' AND note='human decision'", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    assert!(fs::read_dir(&f.refined).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".reprobe-")));
}

#[test]
fn reprobe_stream_failure_never_publishes_partial_csv_or_leaves_temp_file() {
    let root = std::env::temp_dir().join(format!("oviraptor-reprobe-stage-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("P1_active_strong.csv");
    let headers = vec!["asset_key".into(), "host".into(), "link".into()];
    let error = write_reprobe_csv(
        &path,
        &headers,
        [
            Ok(vec![
                "first".into(),
                "first.example.test".into(),
                "https://first.example.test/".into(),
            ]),
            Err("synthetic database iterator failure".into()),
        ],
    )
    .unwrap_err();
    assert!(error.contains("synthetic database iterator failure"));
    assert!(!path.exists());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn resumed_reprobe_counts_csv_records_and_keeps_frozen_bytes_mtime_and_secondary_rows() {
    let f = Fixture::new();
    assert_eq!(f.write().unwrap(), 2);
    let primary = f.refined.join("P1_active_strong.csv");
    let bytes = fs::read(&primary).unwrap();
    let modified = fs::metadata(&primary).unwrap().modified().unwrap();
    let secondary = f.refined.join("P2_strong_needs_validation.csv");
    fs::write(
        &secondary,
        "asset_key,host,link\nlegacy-secondary,legacy.example.test,https://legacy.example.test/\n",
    )
    .unwrap();
    let secondary_bytes = fs::read(&secondary).unwrap();
    fs::remove_file(f.refined.join("P3_name_candidates.csv")).unwrap();
    f.connection
        .execute("DELETE FROM project_assets WHERE asset_id=2", [])
        .unwrap();
    f.connection
        .execute("UPDATE assets SET title='changed after snapshot'", [])
        .unwrap();
    assert_eq!(f.write().unwrap(), 3);
    assert_eq!(fs::read(&primary).unwrap(), bytes);
    assert_eq!(
        fs::metadata(&primary).unwrap().modified().unwrap(),
        modified
    );
    assert_eq!(fs::read(&secondary).unwrap(), secondary_bytes);
    assert_eq!(
        validate_reprobe_source(&f.refined.join("P3_name_candidates.csv"))
            .unwrap()
            .1,
        0
    );
}

#[test]
fn malformed_historical_primary_is_rejected_without_overwriting_or_creating_stages() {
    let f = Fixture::new();
    fs::create_dir_all(&f.refined).unwrap();
    let primary = f.refined.join("P1_active_strong.csv");
    for bytes in [
        "broken,header\nleft,right\n",
        "asset_key,host,link\ntruncated,only-two-fields\n",
    ] {
        fs::write(&primary, bytes).unwrap();
        assert!(f.write().unwrap_err().contains("原复测输入"));
        assert_eq!(fs::read_to_string(&primary).unwrap(), bytes);
        assert!(!f.refined.join("P2_strong_needs_validation.csv").exists());
        assert!(!f.refined.join("P3_name_candidates.csv").exists());
    }
}

#[test]
fn failed_seed_generation_never_replaces_frozen_input_with_a_valid_partial_prefix() {
    let f = Fixture::new();
    let path = f.root.join("seeds.csv");
    let targets = [("domain".into(), "original.example.test".into())];
    write_seeds(&path, &targets).unwrap();
    let before = fs::read(&path).unwrap();
    assert!(write_seeds(
        &path,
        &[
            ("domain".into(), "only-prefix.example.test".into()),
            ("unsupported".into(), "synthetic error".into()),
        ]
    )
    .is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    let missing = f.root.join("new-seeds.csv");
    assert!(write_seeds(
        &missing,
        &[
            ("domain".into(), "only-prefix.example.test".into()),
            ("unsupported".into(), "synthetic error".into()),
        ]
    )
    .is_err());
    assert!(!missing.exists());
    assert!(fs::read_dir(&f.root).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".seeds-")));
}
