#[test]
fn protection_runtime_unknown_sdk_preserves_original_debt_without_publication_or_retry() {
    let mut reply: JsonValue = serde_json::from_str(&proposal_model_response(&json!({"schemaVersion":1,"observed":["original stopped response"],"missing":[],"suggestions":["assess:protection_changed"],"costNotes":[],"risks":[]}).to_string())).unwrap();
    reply.as_object_mut().unwrap().remove("usage");
    let mut f = ProtectionOriginalFixture::with_protection_reply(
        "protection-runtime-unknown",
        Some(reply.to_string()),
    );
    let db = db::open(&f.h.db_path).unwrap();
    let calls = protection_paid_root_calls(&f);
    let requests = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap()
    .consumed;
    let error = multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome)
        .unwrap_err();
    assert_eq!(error, "budget_indeterminate_requires_reconciliation");
    assert_eq!(protection_paid_root_calls(&f), calls + 1);
    let now = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap();
    assert_eq!((now.consumed, now.indeterminate), (requests + 1, 0));
    assert!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_input_tokens"
        )
        .unwrap()
        .indeterminate
            > 0
    );
    assert_eq!(
        protection_paid_phases(&db, &f.actor.root_run_id, "publication"),
        0
    );
    f.assert_closed_read_only(&db, true);
    let paid = super::tests::application_table_snapshot(&db);
    for _ in 0..2 {
        assert!(
            native_coordinator_protection_feedback(&f.h.context, &f.actor, &f.receipt).is_err()
        );
        assert_eq!(super::tests::application_table_snapshot(&db), paid);
        assert_eq!(protection_paid_root_calls(&f), calls + 1);
    }
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn protection_runtime_parallel_feedback_during_first_actual_sdk_is_refused_before_second_charge() {
    type Scope = (
        AgentRunContext,
        crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        String,
    );
    let scope = std::sync::Arc::new(std::sync::Mutex::new(None::<Scope>));
    let observed = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let captured = scope.clone();
    let result = observed.clone();
    let boundary = std::sync::Arc::new(move |_: &Path| {
        let (context, actor, receipt) = captured.lock().unwrap().as_ref().unwrap().clone();
        let db = db::open(&context.db_path).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let error = std::thread::spawn(move || {
            native_coordinator_protection_feedback(&context, &actor, &receipt)
                .err()
                .expect("parallel first SDK must not create a second charge")
        })
        .join()
        .unwrap();
        assert!(
            error.starts_with("protection_closed_sdk_not_idle:"),
            "{error}"
        );
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        *result.lock().unwrap() = Some(error);
    });
    let mut f = ProtectionOriginalFixture::with_protection_boundary(
        "protection-runtime-parallel",
        None,
        Some(boundary),
    );
    *scope.lock().unwrap() = Some((f.h.context.clone(), f.actor.clone(), f.receipt.clone()));
    let db = db::open(&f.h.db_path).unwrap();
    let calls = protection_paid_root_calls(&f);
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    assert!(observed.lock().unwrap().is_some());
    assert_eq!(protection_paid_root_calls(&f), calls + 1);
    assert_eq!(
        protection_paid_phases(&db, &f.actor.root_run_id, "request"),
        1
    );
    assert_eq!(
        protection_paid_phases(&db, &f.actor.root_run_id, "publication"),
        1
    );
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    let before = super::tests::application_table_snapshot(&db);
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(protection_paid_root_calls(&f), calls + 1);
}

#[test]
fn protection_runtime_c_expires_after_actual_paid_sdk_preserving_bill_without_publication() {
    let mut f = ProtectionOriginalFixture::new("protection-runtime-paid-expiry");
    let db = db::open(&f.h.db_path).unwrap();
    let calls = protection_paid_root_calls(&f);
    let requests = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap()
    .consumed;
    let path = f.h.db_path.clone();
    let actor = f.actor.clone();
    ROOT_TICK_AFTER_PAID_TEST_BOUNDARY.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(move || {
            let db = db::open(&path).unwrap();
            assert_eq!(db.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1 AND lease_epoch=?2 AND fencing_token=?3",params![actor.root_run_id,actor.lease_epoch,actor.fencing_token]).unwrap(),1);
        }));
    });
    assert!(
        multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome)
            .is_err()
    );
    ROOT_TICK_AFTER_PAID_TEST_BOUNDARY.with(|slot| assert!(slot.borrow().is_none()));
    assert_eq!(protection_paid_root_calls(&f), calls + 1);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed,
        requests + 1
    );
    assert_eq!(
        protection_paid_phases(&db, &f.actor.root_run_id, "publication"),
        0
    );
    f.assert_closed_read_only(&db, true);
    let before = super::tests::application_table_snapshot(&db);
    for _ in 0..2 {
        assert!(
            native_coordinator_protection_feedback(&f.h.context, &f.actor, &f.receipt).is_err()
        );
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert_eq!(protection_paid_root_calls(&f), calls + 1);
    }
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}
