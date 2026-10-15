use super::*;

struct Fixture {
    root: PathBuf,
    connection: rusqlite::Connection,
}

impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("oviraptor-ownership-accuracy-{}", Uuid::new_v4()));
        let path = db::initialize(&root).unwrap();
        let mut connection = db::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO projects(id,name) VALUES(91,'Synthetic project')",
                [],
            )
            .unwrap();
        save_ownership_profile_for_connection(
            &mut connection,
            AssetOwnershipProfile {
                project_id: 91,
                legal_name: "Synthetic legal entity".into(),
                approved_domains: vec!["approved.example".into()],
                ..Default::default()
            },
            0,
        )
        .unwrap();
        Self { root, connection }
    }

    fn asset(&self, id: i64, host: &str, link: &str, domain: &str) {
        self.connection
            .execute(
                "INSERT INTO assets(id,asset_key,host,link,domain) VALUES(?1,?2,?3,?4,?5)",
                params![id, format!("accuracy-{id}"), host, link, domain],
            )
            .unwrap();
        self.connection
            .execute(
                "INSERT INTO project_assets(project_id,asset_id) VALUES(91,?1)",
                [id],
            )
            .unwrap();
    }

    fn decision(&self, id: i64) -> (String, i64, String, i64) {
        self.connection.query_row("SELECT status,confidence,authorization_status,exposure_eligible FROM asset_ownership_decisions WHERE project_id=91 AND asset_id=?1", [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn ownership_endpoint_host_preserves_dns_labels_and_parses_authority() {
    assert_eq!(
        normalize_ownership_host("HTTPS://WWW.Example.COM.:443/A?q=1"),
        "www.example.com"
    );
    assert_eq!(
        normalize_ownership_host("https://approved.example@evil.example/"),
        "evil.example"
    );
    assert_eq!(normalize_ownership_host("[2001:db8::1]:443"), "2001:db8::1");
    assert_eq!(normalize_ownership_host("2001:db8::1"), "2001:db8::1");
    assert_eq!(
        normalize_ownership_host("https://bücher.example/"),
        "xn--bcher-kva.example"
    );
    assert!(!host_equals_rule("www.example.com", "example.com"));
    assert!(!host_equals_rule("example.com", "www.example.com"));
    assert!(host_equals_rule(
        "HTTPS://WWW.Example.COM:443/A",
        "www.example.com"
    ));
    assert_eq!(
        ownership_endpoint_host("invalid link", "approved.example"),
        ""
    );
    assert!(!ownership_source_domain_conflicts(
        "api.approved.example",
        "approved.example"
    ));
    assert!(ownership_source_domain_conflicts("a.example", "b.example"));
    assert!(ownership_source_domain_conflicts(
        "example.com",
        "www.example.com"
    ));
    assert!(ownership_source_domain_conflicts(
        "192.0.2.1",
        "approved.example"
    ));
}

#[test]
fn assessment_exact_host_does_not_alias_www_and_preserves_manual_or_recycled_assets() {
    let mut f = Fixture::new();
    f.asset(
        1,
        "www.exact.example",
        "https://www.exact.example/",
        "exact.example",
    );
    f.asset(
        2,
        "exact.example",
        "https://exact.example/",
        "exact.example",
    );
    f.asset(
        3,
        "www.exact.example",
        "https://www.exact.example/manual",
        "wrong.example",
    );
    f.asset(
        4,
        "www.exact.example",
        "https://www.exact.example/recycled",
        "wrong.example",
    );
    f.connection.execute_batch("INSERT INTO asset_ownership_rules(project_id,rule_type,pattern,action) VALUES(91,'host','www.exact.example','approve'); INSERT INTO asset_ownership_decisions(project_id,asset_id,status,confidence,authorization_status,exposure_eligible,source,reason) VALUES(91,3,'confirmed',100,'allowed',1,'manual','original manual'),(91,4,'related',70,'revoked',0,'manual','original recycle'); UPDATE project_assets SET is_deleted=1,note='keep recycled',entry_opened_at='old-opened' WHERE asset_id=4").unwrap();
    assess_asset_ownership_for_connection(&mut f.connection, 91).unwrap();
    assert_eq!(
        f.decision(1),
        ("attributed".into(), 92, "unknown".into(), 0)
    );
    assert_eq!(f.decision(2), ("unreviewed".into(), 0, "unknown".into(), 0));
    assert_eq!(
        f.decision(3),
        ("confirmed".into(), 100, "allowed".into(), 1)
    );
    assert_eq!(f.decision(4), ("related".into(), 70, "revoked".into(), 0));
    f.connection
        .execute("UPDATE asset_ownership_rules SET action='exclude'", [])
        .unwrap();
    assess_asset_ownership_for_connection(&mut f.connection, 91).unwrap();
    assert_eq!(f.decision(1).0, "excluded");
    assert_eq!(f.decision(2).0, "unreviewed");
    let recycled: (i64, String, String) = f.connection.query_row("SELECT is_deleted,note,entry_opened_at FROM project_assets WHERE project_id=91 AND asset_id=4", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
    assert_eq!(recycled, (1, "keep recycled".into(), "old-opened".into()));
}

#[test]
fn assessment_source_domain_cannot_replace_endpoint_or_supply_missing_endpoint_evidence() {
    let mut f = Fixture::new();
    f.asset(
        1,
        "foreign.example",
        "https://foreign.example/",
        "approved.example",
    );
    f.asset(
        2,
        "api.approved.example",
        "https://api.approved.example/",
        "approved.example",
    );
    f.asset(
        3,
        "approved.example",
        "https://approved.example@foreign.example/",
        "approved.example",
    );
    f.asset(4, "", "", "approved.example");
    f.asset(5, "approved.example", "invalid link", "approved.example");
    f.asset(6, "approved.example", "https://foreign.example/", "");
    assess_asset_ownership_for_connection(&mut f.connection, 91).unwrap();
    for id in [1, 3, 4, 5, 6] {
        assert_eq!(
            f.decision(id),
            ("unreviewed".into(), 0, "unknown".into(), 0)
        );
        let negative: String = f.connection.query_row("SELECT negative_evidence_json FROM asset_ownership_decisions WHERE project_id=91 AND asset_id=?1", [id], |row| row.get(0)).unwrap();
        assert!(negative.contains("approved.example"));
    }
    assert_eq!(
        f.decision(2),
        ("attributed".into(), 92, "unknown".into(), 0)
    );
}

#[test]
fn actual_group_analysis_keeps_source_domain_conflict_out_of_consistent_confirmation_lane() {
    let mut f = Fixture::new();
    f.asset(
        1,
        "foreign.example",
        "https://foreign.example/",
        "approved.example",
    );
    f.asset(
        2,
        "api.approved.example",
        "https://api.approved.example/",
        "approved.example",
    );
    let run = ownership_review_o2_tests::run_analysis(&mut f.connection, 91, 1);
    let group: (String, String, String) = f.connection.query_row("SELECT g.lane,g.counter_kind,m.host FROM ownership_review_groups g JOIN ownership_review_group_members m ON m.group_id=g.id WHERE m.run_id=?1 AND m.asset_id=1", [run], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
    assert_eq!(group.0, "conflict");
    assert!(group.1.contains("source_domain_mismatch"));
    assert_eq!(group.2, "foreign.example");
    let lane: String = f.connection.query_row("SELECT g.lane FROM ownership_review_groups g JOIN ownership_review_group_members m ON m.group_id=g.id WHERE m.run_id=?1 AND m.asset_id=2", [run], |row| row.get(0)).unwrap();
    assert_eq!(lane, "consistent");
    assert_eq!(f.connection.query_row("SELECT COUNT(*) FROM asset_ownership_decisions WHERE status='confirmed' OR authorization_status='allowed' OR exposure_eligible=1", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
}

#[test]
fn manual_confirmation_learns_actual_www_endpoint_and_never_authorizes() {
    let mut f = Fixture::new();
    f.asset(
        1,
        "www.actual.example",
        "https://www.actual.example/",
        "forged.example",
    );
    let expected = ownership_expectation_of(&f.connection, 91, 1).unwrap();
    let result = confirm_asset_ownership_for_connection(
        &mut f.connection,
        OwnershipConfirmInput {
            selections: vec![AssetSelection {
                project_id: 91,
                asset_id: 1,
            }],
            note: String::new(),
            domain_rule_scope: "exact_host".into(),
        },
        91,
        vec![expected],
    )
    .unwrap();
    assert_eq!(result.learned_rules, vec!["host:www.actual.example"]);
    assert_eq!(
        f.decision(1),
        ("confirmed".into(), 100, "unknown".into(), 0)
    );
    assert_eq!(
        f.connection
            .query_row(
                "SELECT pattern FROM asset_ownership_rules WHERE project_id=91",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "www.actual.example"
    );
}

#[test]
fn old_domain_priority_algorithm_snapshot_is_stale_and_cannot_be_carried_incrementally() {
    let mut f = Fixture::new();
    f.asset(
        1,
        "foreign.example",
        "https://foreign.example/",
        "approved.example",
    );
    f.asset(
        2,
        "manual.example",
        "https://manual.example/",
        "manual.example",
    );
    f.connection.execute("INSERT INTO asset_ownership_decisions(project_id,asset_id,status,confidence,source,authorization_status,exposure_eligible,reason) VALUES(91,2,'confirmed',100,'manual','allowed',1,'original approval')", []).unwrap();
    let run = ownership_review_o2_tests::run_analysis(&mut f.connection, 91, 1);
    f.connection.execute("UPDATE ownership_analysis_runs SET algorithm_version='o2-review-groups-v1' WHERE id=?1", [run]).unwrap();
    let previous = ownership_read_run(&f.connection, run).unwrap().unwrap();
    let current = ownership_current_versions(&f.connection, 91).unwrap();
    assert!(ownership_stale_reasons(&previous, &current).contains(&"algorithm".into()));
    let (next, needs_worker) =
        start_ownership_analysis_with_mode(&mut f.connection, 91, "auto").unwrap();
    assert!(needs_worker);
    assert_eq!(next.run_kind, "full");
    assert!(next.mode_reason.contains("algorithm"));
    assert_eq!(next.base_run_id, None);
    assert_eq!(
        f.decision(2),
        ("confirmed".into(), 100, "allowed".into(), 1),
        "derived-cache invalidation never revokes a manual authorization"
    );
}

fn identifier(entity: i64, kind: &str, value: &str) -> OrgIdentifierDraft {
    OrgIdentifierDraft {
        entity: OrgEntityRef {
            entity_id: Some(entity),
            draft_key: None,
        },
        kind: kind.into(),
        value: value.into(),
        ..Default::default()
    }
}

fn save_identifiers(
    connection: &mut rusqlite::Connection,
    identifiers: Vec<OrgIdentifierDraft>,
) -> Result<ProjectOwnershipDirectorySaveResult, String> {
    let version = org_directory_version(connection)?;
    save_project_ownership_directory_for_connection(
        connection,
        ProjectOwnershipDirectorySave {
            project_id: 91,
            expected_directory_version: version,
            expected_binding_version: 0,
            identifiers,
            ..Default::default()
        },
    )
}

#[test]
fn historical_raw_www_directory_keys_are_reused_without_deleting_or_colliding_with_new_apex() {
    let mut f = Fixture::new();
    f.asset(
        1,
        "www.same.example",
        "https://www.same.example/",
        "same.example",
    );
    f.connection.execute_batch("INSERT INTO org_entities(id,entity_kind,legal_name,normalized_name) VALUES(700,'group','Directory Group','DirectoryGroup'),(701,'group','Other Group','OtherGroup'); INSERT INTO org_entity_identifiers(id,entity_id,kind,value,normalized_value) VALUES(700,700,'official_domain','www.same.example','same.example'),(701,700,'official_domain','www.split.example','split.example'); INSERT INTO asset_ownership_decisions(project_id,asset_id,status,confidence,source,authorization_status,exposure_eligible,reason) VALUES(91,1,'confirmed',100,'manual','allowed',1,'original approval')").unwrap();
    for _ in 0..2 {
        let saved = save_identifiers(
            &mut f.connection,
            vec![identifier(
                700,
                "official_domain",
                "https://WWW.same.example:443/path",
            )],
        )
        .unwrap();
        assert_eq!(
            saved.created_identifiers, 0,
            "reuse is explicit and does not create a second identifier"
        );
    }
    let old: (String, String) = f
        .connection
        .query_row(
            "SELECT value,normalized_value FROM org_entity_identifiers WHERE id=700",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(old, ("www.same.example".into(), "same.example".into()));
    let saved = save_identifiers(
        &mut f.connection,
        vec![
            identifier(700, "official_domain", "same.example"),
            identifier(701, "official_domain", "split.example"),
        ],
    )
    .unwrap();
    assert_eq!(
        saved.created_identifiers, 2,
        "apex and www are independent explicit hosts even with an old collapsed key"
    );
    assert_eq!(f.connection.query_row("SELECT normalized_value FROM org_entity_identifiers WHERE entity_id=700 AND value='same.example'", [], |row| row.get::<_,String>(0)).unwrap(), "host-v2:same.example");
    let count: i64 = f
        .connection
        .query_row("SELECT COUNT(*) FROM org_entity_identifiers", [], |row| {
            row.get(0)
        })
        .unwrap();
    let version = org_directory_version(&f.connection).unwrap();
    assert!(save_identifiers(
        &mut f.connection,
        vec![identifier(701, "official_domain", "www.same.example")]
    )
    .unwrap_err()
    .starts_with(ORG_DIRECTORY_INVALID));
    assert_eq!(org_directory_version(&f.connection).unwrap(), version);
    assert_eq!(
        f.connection
            .query_row("SELECT COUNT(*) FROM org_entity_identifiers", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        count
    );
    assert_eq!(
        f.decision(1),
        ("confirmed".into(), 100, "allowed".into(), 1)
    );
}

#[test]
fn directory_duplicate_raw_host_aliases_do_not_invent_multiple_entities_and_shared_v2_still_blocks_generalization(
) {
    let mut f = Fixture::new();
    f.asset(
        1,
        "www.same.example",
        "https://www.same.example/",
        "same.example",
    );
    f.asset(
        2,
        "tenant.shared.example",
        "https://tenant.shared.example/",
        "shared.example",
    );
    f.connection.execute_batch("INSERT INTO org_entities(id,entity_kind,legal_name,normalized_name) VALUES(700,'group','Directory Group','DirectoryGroup'); INSERT INTO org_entity_identifiers(entity_id,kind,value,normalized_value) VALUES(700,'official_domain','www.same.example','same.example'),(700,'official_domain','www.same.example','host-v2:www.same.example');").unwrap();
    let saved = save_identifiers(
        &mut f.connection,
        vec![identifier(700, "shared_domain", "tenant.shared.example")],
    )
    .unwrap();
    assert_eq!(saved.created_identifiers, 1);
    f.connection.execute("INSERT INTO project_org_bindings(project_id,project_kind,group_entity_id,binding_version) VALUES(91,'group_ledger',700,1)", []).unwrap();
    let run = ownership_review_o2_tests::run_analysis(&mut f.connection, 91, 1);
    let (lane,counter): (String,String) = f.connection.query_row("SELECT g.lane,g.counter_kind FROM ownership_review_groups g JOIN ownership_review_group_members m ON m.group_id=g.id WHERE m.run_id=?1 AND m.asset_id=1", [run], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(lane, "consistent");
    assert!(!counter.contains("multiple_entities"));
    let before: i64 = f
        .connection
        .query_row("SELECT COUNT(*) FROM asset_ownership_rules", [], |row| {
            row.get(0)
        })
        .unwrap();
    let error =
        ownership_learn_host_rule(&f.connection, 91, 2, "approve", "subdomains").unwrap_err();
    assert!(error.starts_with(OWNERSHIP_RULE_SCOPE_DENIED));
    assert_eq!(
        f.connection
            .query_row("SELECT COUNT(*) FROM asset_ownership_rules", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        before
    );
}

#[test]
fn directory_ignored_unique_key_collision_is_an_error_and_rolls_back_the_whole_save() {
    let mut f = Fixture::new();
    f.connection.execute_batch("INSERT INTO org_entities(id,entity_kind,legal_name,normalized_name) VALUES(700,'group','Directory Group','DirectoryGroup'); INSERT INTO org_entity_identifiers(entity_id,kind,value,normalized_value) VALUES(700,'official_domain','unrelated.example','host-v2:new.example');").unwrap();
    let version = org_directory_version(&f.connection).unwrap();
    let changes: i64 = f
        .connection
        .query_row("SELECT COUNT(*) FROM org_directory_changes", [], |row| {
            row.get(0)
        })
        .unwrap();
    let error = save_identifiers(
        &mut f.connection,
        vec![
            identifier(700, "official_domain", "first.example"),
            identifier(700, "official_domain", "new.example"),
        ],
    )
    .unwrap_err();
    assert!(error.starts_with(ORG_DIRECTORY_CONFLICT));
    assert_eq!(org_directory_version(&f.connection).unwrap(), version);
    assert_eq!(
        f.connection
            .query_row("SELECT COUNT(*) FROM org_entity_identifiers", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        f.connection
            .query_row("SELECT COUNT(*) FROM org_directory_changes", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        changes
    );
}

#[test]
fn legacy_learned_patterns_are_not_guessed_or_promoted_but_explicit_reconfirmation_versions_the_rule(
) {
    let mut f = Fixture::new();
    f.asset(
        1,
        "actual.example",
        "https://actual.example/",
        "actual.example",
    );
    f.asset(
        2,
        "www.actual.example",
        "https://www.actual.example/",
        "actual.example",
    );
    f.asset(
        3,
        "actual.example",
        "https://actual.example/manual",
        "actual.example",
    );
    f.asset(
        4,
        "sub.origin.example",
        "https://sub.origin.example/",
        "origin.example",
    );
    f.connection.execute_batch("INSERT INTO asset_ownership_rules(project_id,rule_type,pattern,action,source) VALUES(91,'host','actual.example','approve','manual_decision'),(91,'domain','origin.example','approve','manual_decision'); INSERT INTO asset_ownership_decisions(project_id,asset_id,status,confidence,source,authorization_status,exposure_eligible,reason) VALUES(91,3,'confirmed',100,'manual','allowed',1,'original approval')").unwrap();
    assess_asset_ownership_for_connection(&mut f.connection, 91).unwrap();
    for id in [1, 2, 4] {
        assert_eq!(
            f.decision(id),
            ("unreviewed".into(), 0, "unknown".into(), 0)
        );
    }
    let run = ownership_review_o2_tests::run_analysis(&mut f.connection, 91, 1);
    for id in [1, 2, 4] {
        let (lane,counter): (String,String) = f.connection.query_row("SELECT g.lane,g.counter_kind FROM ownership_review_groups g JOIN ownership_review_group_members m ON m.group_id=g.id WHERE m.run_id=?1 AND m.asset_id=?2", params![run,id], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_ne!(lane, "consistent");
        assert!(counter.contains("legacy_learned_rule_unverified"));
    }
    assert_eq!(
        f.decision(3),
        ("confirmed".into(), 100, "allowed".into(), 1)
    );
    let expected = ownership_expectation_of(&f.connection, 91, 1).unwrap();
    let confirmed = confirm_asset_ownership_for_connection(
        &mut f.connection,
        OwnershipConfirmInput {
            selections: vec![AssetSelection {
                project_id: 91,
                asset_id: 1,
            }],
            note: String::new(),
            domain_rule_scope: "exact_host".into(),
        },
        91,
        vec![expected],
    )
    .unwrap();
    assert_eq!(confirmed.learned_rules, vec!["host:actual.example"]);
    let source:String = f.connection.query_row("SELECT source FROM asset_ownership_rules WHERE project_id=91 AND rule_type='host' AND pattern='actual.example'", [], |row| row.get(0)).unwrap();
    assert_eq!(source, OWNERSHIP_LEARNED_RULE_SOURCE_V2);
    assess_asset_ownership_for_connection(&mut f.connection, 91).unwrap();
    assert_eq!(
        f.decision(1),
        ("confirmed".into(), 100, "unknown".into(), 0)
    );
    assert_eq!(f.decision(2), ("unreviewed".into(), 0, "unknown".into(), 0));
    assert_eq!(
        f.decision(3),
        ("confirmed".into(), 100, "allowed".into(), 1)
    );
    assert_eq!(
        f.connection
            .query_row(
                "SELECT COUNT(*) FROM asset_ownership_rules WHERE pattern LIKE '%www%'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0,
        "no guessed www migration"
    );
}

#[test]
fn shared_ip_and_network_organization_never_transfer_ownership_confirmation_or_authorization() {
    let mut f = Fixture::new();
    f.asset(
        1,
        "api.approved.example",
        "https://api.approved.example/",
        "approved.example",
    );
    f.asset(
        2,
        "unrelated.example",
        "https://unrelated.example/",
        "unrelated.example",
    );
    f.asset(3, "192.0.2.1", "http://192.0.2.1/", "");
    f.connection
        .execute("UPDATE assets SET ip='192.0.2.1'", [])
        .unwrap();
    f.connection
        .execute(
            "UPDATE assets SET company='AS64500',extra_json=?1 WHERE id=3",
            [serde_json::json!({"org":"Synthetic legal entity"}).to_string()],
        )
        .unwrap();
    assess_asset_ownership_for_connection(&mut f.connection, 91).unwrap();
    assert_eq!(
        f.decision(1),
        ("attributed".into(), 92, "unknown".into(), 0)
    );
    assert_eq!(f.decision(2), ("unreviewed".into(), 0, "unknown".into(), 0));
    let network = f.decision(3);
    assert_eq!(
        (network.0.as_str(), network.2.as_str(), network.3),
        ("attributed", "unknown", 0)
    );
    assert!(network.1 <= 55);
    for id in [1, 2, 3] {
        let expected = ownership_expectation_of(&f.connection, 91, id).unwrap();
        let error = authorize_asset_ownership_for_connection(
            &mut f.connection,
            OwnershipAuthorizeInput {
                selections: vec![AssetSelection {
                    project_id: 91,
                    asset_id: id,
                }],
                note: String::new(),
            },
            91,
            vec![expected],
        )
        .unwrap_err();
        assert!(error.starts_with(OWNERSHIP_AUTHORIZATION_REQUIRES_CONFIRMED));
    }
    assert_eq!(f.connection.query_row("SELECT COUNT(*) FROM asset_ownership_decisions WHERE status='confirmed' OR authorization_status='allowed' OR exposure_eligible<>0", [], |row| row.get::<_,i64>(0)).unwrap(),0);
}
