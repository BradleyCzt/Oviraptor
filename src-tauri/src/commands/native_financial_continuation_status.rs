// Only original immutable birth, paid publication and exit proofs establish
// completion. Mutable status text can stop work, never manufacture completion.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum FinancialContinuationStatusKind {
    Issued,
    AssessmentPublished,
    ReviewRequired,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FinancialContinuationTaskDescription {
    purpose: String,
    target_key: String,
    original_plan_hash: String,
    original_semantic_replay_allowed: bool,
    limits: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FinancialContinuationStatus {
    schema: u8,
    approval_id: String,
    proposal_hash: String,
    scan_id: String,
    source_attempt_number: i64,
    new_attempt_number: i64,
    source_root_run_id: String,
    new_root_run_id: String,
    source_receipt_id: String,
    status: FinancialContinuationStatusKind,
    assessment_completed: bool,
    attempt_closed: bool,
    automatic_resume_allowed: bool,
    original_semantic_replay_allowed: bool,
    financial_source: JsonValue,
    task_description: FinancialContinuationTaskDescription,
    original_deadline: String,
    review_reason: Option<String>,
    worker_start_allowed: bool,
}

fn financial_status_string(fact: &JsonValue, key: &str) -> Result<String, String> {
    fact[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "financial_continuation_status_invalid".into())
}

fn financial_continuation_status_in(
    db: &rusqlite::Connection,
    fact: &JsonValue,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::multi_agent::budget;
    if db.is_autocommit() {
        return Err("financial_continuation_requires_snapshot".into());
    }
    let root = financial_status_string(fact, "newRootRunId")?;
    let scan = financial_status_string(fact, "scanId")?;
    if verify_financial_continuation_birth_on(db, &root)? != *fact {
        return Err("financial_continuation_status_birth_changed".into());
    }
    let declaration = budget::root_definition::read(db, &root)?
        .ok_or("financial_continuation_status_budget_missing")?;
    if declaration.schema_version != 3 {
        return Err("financial_continuation_status_budget_invalid".into());
    }
    budget::root::RootOwner::load_original(db, &root)?;
    let published = native_financial_continuation_published_in(db, &scan, &root)?.is_some();
    let new_attempt = fact["newAttemptNumber"]
        .as_i64()
        .ok_or("financial_continuation_status_invalid")?;
    let (root_state,root_code,root_finished,attempt_state,stop,attempt_checkpoint,attempt_finished):
        (String,String,String,String,String,String,String) = db.query_row(
        "SELECT r.status,r.terminal_code,r.finished_at,a.status,a.stop_reason,a.checkpoint,a.finished_at
         FROM agent_runs r JOIN sentinel_scan_attempts a ON a.scan_id=r.scan_id
         AND a.attempt_number=r.attempt_number WHERE r.id=?1 AND r.scan_id=?2 AND r.attempt_number=?3",
        params![root,scan,new_attempt],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)))
        .map_err(|_|"financial_continuation_status_scope_missing")?;
    let exit: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_multi_exit_receipts WHERE root_run_id=?1)",
            [&root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let claims_closed = root_state == "terminal"
        || exit
        || stop == "financial_continuation_assessment_only"
        || attempt_checkpoint == "financial_assessment_published";
    let mut closed = false;
    if claims_closed {
        if !published
            || root_state != "terminal"
            || root_code != "financial_continuation_assessment_only"
            || root_finished.is_empty()
            || attempt_state != "partial"
            || attempt_finished.is_empty()
            || stop != "financial_continuation_assessment_only"
            || attempt_checkpoint != "financial_assessment_published"
        {
            return Err("financial_continuation_status_closure_unproven".into());
        }
        budget::clock::FinalClock::verify_original_exit(db, &root)?;
        closed = true;
    }
    let (current, state, checkpoint): (i64, String, String) = db
        .query_row(
            "SELECT attempt_count,status,current_checkpoint FROM sentinel_scans WHERE id=?1
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
            [&scan],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| "financial_continuation_status_scope_missing")?;
    let mut review = None;
    let mut runnable = false;
    if !closed {
        if attempt_state == "partial" && stop == "financial_assessment_requires_review" {
            review = Some("financial_assessment_requires_review".into());
        } else if current != new_attempt
            || state != "scanning"
            || attempt_state != "scanning"
            || !attempt_finished.is_empty()
            || !stop.is_empty()
            || !matches!(root_state.as_str(), "prepared" | "running")
            || checkpoint == "financial_assessment_requires_review"
        {
            review = Some("financial_continuation_attempt_not_runnable".into());
        } else {
            let db_path = db
                .path()
                .filter(|p| !p.is_empty())
                .ok_or("financial_continuation_status_database_missing")?;
            // This is current eligibility for the dedicated existing worker,
            // never a grant obtained from mutable status or remaining balances.
            let authority = native_financial_continuation_require_authority_in(db, fact)
                .and_then(|_| {
                    native_financial_continuation_context_in(db, Path::new(db_path), &scan, &root)
                })
                .and_then(|_| budget::clock::remaining(db, &root).map(|_| ()));
            if authority.is_err() {
                review = Some("authority_revalidation_required".into());
            } else if !published
                && native_financial_continuation_require_admission_in(
                    db,
                    Path::new(db_path),
                    &scan,
                    &root,
                )
                .is_err()
            {
                review = Some("financial_continuation_admission_requires_review".into());
            } else {
                runnable = true;
            }
        }
    }
    let status = if review.is_some() {
        FinancialContinuationStatusKind::ReviewRequired
    } else if published {
        FinancialContinuationStatusKind::AssessmentPublished
    } else {
        FinancialContinuationStatusKind::Issued
    };
    let description: FinancialContinuationTaskDescription =
        serde_json::from_value(fact["taskDescription"].clone())
            .map_err(|_| "financial_continuation_status_description_invalid")?;
    if description.purpose != "fresh_root_assessment_of_frozen_web_evidence"
        || description.limits != "remaining_original_budget_and_deadline"
        || description.original_semantic_replay_allowed
        || description.target_key != fact["financialSource"]["targetKey"]
        || description.original_plan_hash != fact["proposal"]["taskDescription"]["originalPlanHash"]
    {
        return Err("financial_continuation_status_description_invalid".into());
    }
    let result = FinancialContinuationStatus {
        schema: 1,
        approval_id: financial_status_string(fact, "approvalId")?,
        proposal_hash: financial_status_string(fact, "proposalHash")?,
        scan_id: scan,
        source_attempt_number: fact["sourceAttemptNumber"]
            .as_i64()
            .ok_or("financial_continuation_status_invalid")?,
        new_attempt_number: new_attempt,
        source_root_run_id: financial_status_string(fact, "sourceRootRunId")?,
        new_root_run_id: root,
        source_receipt_id: financial_status_string(fact, "sourceReceiptId")?,
        status,
        assessment_completed: published,
        attempt_closed: closed,
        automatic_resume_allowed: false,
        original_semantic_replay_allowed: false,
        financial_source: fact["financialSource"].clone(),
        task_description: description,
        original_deadline: financial_status_string(fact, "originalDeadline")?,
        review_reason: review,
        worker_start_allowed: runnable,
    };
    serde_json::to_value(result).map_err(|e| e.to_string())
}

fn native_financial_continuation_status_on(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
    source: &str,
    receipt: &str,
    proposal: &str,
) -> Result<JsonValue, String> {
    if db.is_autocommit() {
        let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
        return native_financial_continuation_status_on(
            &snapshot, scan, attempt, source, receipt, proposal,
        );
    }
    let root: String = db
        .query_row(
            "SELECT new_root_run_id FROM agent_financial_continuations WHERE source_root_run_id=?1",
            [source],
            |r| r.get(0),
        )
        .map_err(|_| "financial_continuation_issuance_missing")?;
    let fact = verify_financial_continuation_birth_on(db, &root)?;
    if fact["scanId"] != scan
        || fact["sourceAttemptNumber"] != attempt
        || fact["sourceRootRunId"] != source
        || fact["sourceReceiptId"] != receipt
        || fact["proposalHash"] != proposal
    {
        return Err("financial_continuation_status_binding_conflict".into());
    }
    financial_continuation_status_in(db, &fact)
}

#[tauri::command]
pub fn get_native_root_financial_continuation(
    state: State<'_, AppState>,
    scan_id: String,
    attempt_number: i64,
    root_run_id: String,
    receipt_id: String,
    proposal_hash: String,
) -> Result<JsonValue, String> {
    // Status observation must not create a database or run schema/seed writers.
    let db = rusqlite::Connection::open_with_flags(
        &state.db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    db.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    native_financial_continuation_status_on(
        &db,
        &scan_id,
        attempt_number,
        &root_run_id,
        &receipt_id,
        &proposal_hash,
    )
}
