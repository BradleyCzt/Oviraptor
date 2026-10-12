// Ceilings are frozen at the original Root's birth. These cases never tamper
// with a cap to simulate exhaustion; the real paid bill consumes 50,010 tokens.
#[test]
fn financial_continuation_legitimate_exhaustion_and_positive_insufficient_remainder_refuse_new_work(
) {
    let _real = RealSpecialistTransport::enter();
    for (cap, remaining, code) in [
        (50_010, 0, "financial_continuation_budget_exhausted"),
        (50_011, 1, "child_budget_reservation_exceeded_or_stale"),
    ] {
        let (f, seen, receipt) = financial_issuance_fixture_with_limits(
            &format!("original-legitimate-limit-{cap}"),
            (cap, 20),
        );
        let db = db::open(&f.context.db_path).unwrap();
        let preview = finance_preview_read(&f, &db, &receipt).unwrap();
        assert_eq!(
            preview["financialSource"]["sharedRemainingTokens"],
            remaining
        );
        let original = financial_guard_originals(&f, &db);
        assert_eq!(
            financial_guard_approval_error(
                &f,
                &db,
                &receipt,
                preview["proposalHash"].as_str().unwrap(),
            ),
            code
        );
        financial_guard_verify_originals(&f, &db, &seen, &original);
        assert_eq!(
            db.query_row(
                "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
                [&f.context.scan_id],
                |r| r.get::<_, i64>(0),
            )
            .unwrap(),
            1
        );
        // A repeated explicit click cannot mint a new allowance or send SDK.
        assert_eq!(
            financial_guard_approval_error(
                &f,
                &db,
                &receipt,
                preview["proposalHash"].as_str().unwrap(),
            ),
            code
        );
        financial_guard_verify_originals(&f, &db, &seen, &original);
    }
}
