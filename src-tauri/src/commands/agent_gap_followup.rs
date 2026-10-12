// Follow-up preparation reads local evidence only. No lease, capability,
// credential or target request is inherited from the source task.
fn checked_evidence_directory(attempt_dir: &Path, target_dir: &Path, scan_id: &str) -> Result<PathBuf, String> {
    let work = attempt_dir.canonicalize().map_err(|_| "followup_attempt_directory_missing")?;
    let name = target_dir.file_name().and_then(|s| s.to_str()).ok_or("followup_target_directory_invalid")?;
    let position = name.strip_prefix("target-").ok_or("followup_target_directory_invalid")?;
    if position.len() < 5 || !position.bytes().all(|b| b.is_ascii_digit()) {
        return Err("followup_target_directory_invalid".into());
    }
    let expected = work.join("url-pipeline").join(name);
    let actual = target_dir.canonicalize().map_err(|_| "followup_evidence_missing_requires_fresh_task")?;
    // Canonical comparison against the *unresolved* suffix rejects a symlink
    // for either url-pipeline or target-N, even one pointing inside the attempt.
    if actual != expected || !actual.is_dir() {
        return Err("followup_evidence_directory_mismatch".into());
    }
    let marker = actual.join(".oviraptor-scan-id");
    let metadata = fs::symlink_metadata(&marker).map_err(|_| "followup_evidence_marker_missing")?;
    if !metadata.is_file() || metadata.len() > 256
        || fs::read_to_string(marker).map_err(|_| "followup_evidence_marker_unreadable")? != scan_id {
        return Err("followup_evidence_marker_mismatch".into());
    }
    Ok(actual)
}

fn bind_agent_evidence_location(context: &AgentRunContext) -> Result<(), String> {
    let run = context.run.as_ref().ok_or("followup_root_run_missing")?;
    let connection = db::open(&context.db_path)?;
    let transaction = rusqlite::Transaction::new_unchecked(&connection, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| format!("followup_location_lock:{e}"))?;
    let attempt_dir: String = transaction.query_row(
        "SELECT a.work_dir FROM sentinel_scan_attempts a JOIN agent_runs r \
         ON r.scan_id=a.scan_id AND r.attempt_number=a.attempt_number \
         JOIN sentinel_scans s ON s.id=a.scan_id AND s.attempt_count=a.attempt_number \
         WHERE r.id=?1 AND r.scan_id=?2 AND r.attempt_number=?3 AND r.target_url=?4 \
         AND r.role='coordinator' AND r.parent_run_id IS NULL AND r.status IN ('prepared','running') \
         AND r.backend='native' AND s.status='scanning' AND a.status='scanning'",
        params![run.run_id, context.scan_id, context.attempt_number, context.target_url], |r| r.get(0),
    ).map_err(|_| "followup_active_attempt_unavailable")?;
    let attempt = Path::new(&attempt_dir).canonicalize().map_err(|_| "followup_attempt_directory_missing")?;
    let target = checked_evidence_directory(&attempt, &context.target_dir, &context.scan_id)?;
    transaction.execute(
        "INSERT INTO agent_evidence_locations(root_run_id,scan_id,attempt_number,target_url,attempt_dir,target_dir) \
         VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(root_run_id) DO NOTHING",
        params![run.run_id, context.scan_id, context.attempt_number, context.target_url, attempt.to_string_lossy(), target.to_string_lossy()],
    ).map_err(|e| format!("followup_location_insert:{e}"))?;
    let resolved = resolve_agent_evidence_location(&transaction, &run.run_id)?;
    if resolved != target { return Err("followup_evidence_binding_conflict".into()); }
    transaction.commit().map_err(|e| format!("followup_location_commit:{e}"))
}

pub(crate) fn resolve_agent_evidence_location(connection: &rusqlite::Connection, root: &str) -> Result<PathBuf, String> {
    let (scan, stored_attempt, stored_target, current_attempt): (String, String, String, String) = connection.query_row(
        "SELECT l.scan_id,l.attempt_dir,l.target_dir,a.work_dir FROM agent_evidence_locations l \
         JOIN agent_runs r ON r.id=l.root_run_id AND r.scan_id=l.scan_id \
           AND r.attempt_number=l.attempt_number AND r.target_url=l.target_url \
         JOIN sentinel_scan_attempts a ON a.scan_id=l.scan_id AND a.attempt_number=l.attempt_number \
         WHERE l.root_run_id=?1 AND r.role='coordinator' AND r.parent_run_id IS NULL AND r.backend='native'",
        [root], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).map_err(|_| "followup_evidence_binding_missing_requires_fresh_task")?;
    let current = Path::new(&current_attempt).canonicalize().map_err(|_| "followup_attempt_directory_missing")?;
    if current != Path::new(&stored_attempt) { return Err("followup_attempt_directory_changed".into()); }
    let resolved = checked_evidence_directory(&current, Path::new(&stored_target), &scan)?;
    if resolved != Path::new(&stored_target) { return Err("followup_evidence_directory_changed".into()); }
    Ok(resolved)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GapFollowupPreview {
    source_scan_id: String,
    assessment_message_id: String,
    project_id: i64,
    target_url: String,
    root_run_id: String,
    assignment_id: String,
    candidate_id: String,
    candidate_revision: i64,
    missing_evidence: JsonValue,
    proposal: JsonValue,
    source_hash: String,
    target_requests_granted: u64,
}

fn gap_followup_preview_in(connection: &rusqlite::Connection, scan_id: &str, message_id: &str) -> Result<GapFollowupPreview, String> {
    if connection.is_autocommit() { return Err("gap_original_snapshot_required".into()); }
    let (root, assignment, child, target, project, raw): (String,String,String,String,i64,String) = connection.query_row(
        "SELECT m.root_run_id,m.assignment_id,m.to_run_id,r.target_url,s.project_id,m.payload_json \
         FROM agent_messages m JOIN agent_runs r ON r.id=m.root_run_id \
         JOIN sentinel_scans s ON s.id=r.scan_id JOIN projects p ON p.id=s.project_id \
         JOIN agent_assignments a ON a.id=m.assignment_id AND a.coordinator_run_id=r.id AND a.child_run_id=m.to_run_id \
         WHERE m.id=?1 AND s.id=?2 AND s.scan_type='web' AND p.status='active' \
         AND a.state='completed' AND a.role='deep_investigator' \
         AND m.kind='proposal_assessed' AND m.from_run_id=r.id AND m.delivered_at<>'' AND m.acknowledged_at<>'' \
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id)",
        params![message_id, scan_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
    ).map_err(|_| "followup_acknowledged_assessment_unavailable")?;
    let assessment: JsonValue = serde_json::from_str(&raw).map_err(|_| "followup_assessment_invalid")?;
    if assessment["schemaVersion"] != 3 || assessment["newAttemptRequired"] != true
        || assessment["targetRequestsGranted"] != 0 || assessment["reasonCode"] != "operator_approval_new_attempt_required" {
        return Err("followup_new_task_not_requested".into());
    }
    let candidate = assessment["candidateId"].as_str().ok_or("followup_candidate_missing")?;
    let revision = assessment["evidenceRevision"].as_i64().ok_or("followup_revision_missing")?;
    let missing_text: String = connection.query_row(
        "SELECT d.missing_evidence_json FROM agent_review_requests q JOIN agent_review_decisions d ON d.id=q.decision_id \
         WHERE q.root_run_id=?1 AND q.candidate_id=?2 AND q.candidate_revision=?3",
        params![root, candidate, revision], |r| r.get(0),
    ).map_err(|_| "followup_review_missing")?;
    let missing: JsonValue = serde_json::from_str(&missing_text).map_err(|_| "followup_missing_evidence_invalid")?;
    let dir = resolve_agent_evidence_location(connection, &root)?;
    let (sealed, _, _) = sealed_gap_review_candidate_in_transaction(connection, &root, candidate, revision, &missing, &dir)?;
    if !completed_gap_round_valid(connection, &root, &assignment, &child, candidate, revision, &missing, &dir)? {
        return Err("followup_gap_round_incomplete".into());
    }
    let proposal_text: String = connection.query_row(
        "SELECT payload_json FROM agent_messages WHERE root_run_id=?1 AND assignment_id=?2 AND kind='gap_proposed'",
        params![root, assignment], |r| r.get(0),
    ).map_err(|_| "followup_proposal_missing")?;
    let proposal: JsonValue = serde_json::from_str(&proposal_text).map_err(|_| "followup_proposal_invalid")?;
    let manifest = review_snapshot_manifest(connection, &root, &dir, &sealed.to_string())?;
    let hash = crate::agent_runtime::store::stable_hash(&json!({
        "sourceScanId":scan_id,"messageId":message_id,"root":root,"assignment":assignment,
        "project":project,"target":target,"assessment":assessment,"proposal":proposal,"manifest":manifest,
    }).to_string());
    Ok(GapFollowupPreview { source_scan_id:scan_id.into(), assessment_message_id:message_id.into(),
        project_id:project, target_url:target, root_run_id:root, assignment_id:assignment,
        candidate_id:candidate.into(), candidate_revision:revision,
        missing_evidence:crate::agent_runtime::secrets::redact_json(&missing),
        proposal:crate::agent_runtime::secrets::redact_json(&proposal["proposal"]), source_hash:hash, target_requests_granted:0 })
}

#[tauri::command]
pub fn preview_agent_gap_followup(state: State<AppState>, scan_id: String, assessment_message_id: String) -> Result<GapFollowupPreview, String> {
    let connection = db::open(&state.db_path)?;
    let transaction = connection.unchecked_transaction().map_err(|e| e.to_string())?;
    gap_followup_preview_in(&transaction, &scan_id, &assessment_message_id)
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GapFollowupDraftInput {
    request_id: String,
    source_scan_id: String,
    assessment_message_id: String,
    source_hash: String,
    task_name: String,
    scan_mode: Option<String>,
    max_budget_usd: Option<f64>,
    auth_session_ids: Vec<String>,
    auth_session_scope_id: String,
    skill_ids: Vec<i64>,
    instruction: String,
    closure: Option<String>,
}

fn create_agent_gap_followup_in(connection: &rusqlite::Connection, input: &GapFollowupDraftInput) -> Result<SentinelScan, String> {
    if Uuid::parse_str(&input.request_id).is_err() { return Err("followup_request_id_invalid".into()); }
    if input.max_budget_usd.is_some_and(|v| !v.is_finite()) { return Err("followup_budget_invalid".into()); }
    let request_hash = crate::agent_runtime::store::stable_hash(&serde_json::to_string(input).map_err(|e| e.to_string())?);
    let transaction = rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| format!("followup_draft_lock:{e}"))?;
    let saved: Option<(String,String)> = transaction.query_row(
        "SELECT scan_id,request_hash FROM agent_gap_followups WHERE request_id=?1", [&input.request_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ).optional().map_err(|e| e.to_string())?;
    if let Some((scan_id, saved_hash)) = saved {
        if saved_hash != request_hash { return Err("followup_request_id_conflict".into()); }
        // Receipt replay returns the actual current task, even after it started.
        // It never captures identities again or restarts an existing task.
        let scan=sentinel_scan_by_id(&transaction, &scan_id)?;
        record_gap_submission_result(&transaction,&input.request_id,&scan_id)?;
        transaction.commit().map_err(|e|format!("followup_draft_commit:{e}"))?;
        return Ok(scan);
    }
    // A user may have cancelled an uncommitted durable submission while this
    // invocation was waiting for SQLite. Check inside the creation transaction.
    let cancelled: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_gap_followup_submissions WHERE request_id=?1 AND (state='released' OR created_scan_id<>''))",
        [&input.request_id], |r| r.get(0),
    ).map_err(|e|e.to_string())?;
    if cancelled { return Err("followup_submission_released".into()); }
    let preview = gap_followup_preview_in(&transaction, &input.source_scan_id, &input.assessment_message_id)?;
    if preview.source_hash != input.source_hash { return Err("followup_source_changed_preview_again".into()); }
    let mode_window=crate::agent_runtime::web_mode::draft::DraftCreationWindow::begin(&transaction)?;
    let scan = create_sentinel_url_draft_rows(
        &transaction, preview.project_id, input.task_name.clone(), vec![preview.target_url.clone()],
        input.scan_mode.clone(), input.max_budget_usd, None, Some(input.auth_session_ids.clone()),
        Some(input.auth_session_scope_id.clone()), Some(input.skill_ids.clone()), Some(input.instruction.clone()), input.closure.clone(), Some(&preview.target_url),
    )?;
    let changed = transaction.execute(
        "INSERT INTO agent_gap_followups(scan_id,request_id,request_hash,source_scan_id,assessment_message_id,source_hash,source_preview_json) \
         VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![scan.id, input.request_id, request_hash, preview.source_scan_id, preview.assessment_message_id,
            preview.source_hash, serde_json::to_string(&preview).map_err(|e| e.to_string())?],
    ).map_err(|e| format!("followup_provenance_insert:{e}"))?;
    if changed != 1 { return Err("followup_provenance_not_saved".into()); }
    record_gap_submission_result(&transaction,&input.request_id,&scan.id)?;
    let mode=crate::agent_runtime::web_mode::draft::register_new(mode_window,&scan.id,crate::agent_runtime::web_mode::WebMode::Multi)?;
    crate::agent_runtime::web_mode::action::freeze_new(mode,&crate::agent_runtime::web_mode::action::NewWebAction {
        schema_version:1,kind:"gap_followup".into(),request_id:input.request_id.clone(),source_scan_id:input.source_scan_id.clone(),
        source_hash:input.source_hash.clone(),request_hash:request_hash.clone(),targets:vec![preview.target_url.clone()],
    })?;
    let final_preview=gap_followup_preview_in(&transaction,&input.source_scan_id,&input.assessment_message_id)?;
    if final_preview.source_hash!=preview.source_hash {return Err("web_mode_gap_source_changed_during_creation".into());}
    transaction.commit().map_err(|e| format!("followup_draft_commit:{e}"))?;
    Ok(scan)
}

#[tauri::command]
pub fn create_agent_gap_followup(state: State<AppState>, input: GapFollowupDraftInput) -> Result<SentinelScan, String> {
    submit_agent_gap_followup_in(&db::open(&state.db_path)?, &input)
}

fn gap_followup_control_preflight(connection: &rusqlite::Connection, app_data_dir: &Path, scan_id: &str) -> Result<Option<i64>, String> {
    let stored: Option<String> = connection.query_row(
        "SELECT source_preview_json FROM agent_gap_followups WHERE scan_id=?1", [scan_id], |r| r.get(0),
    ).optional().map_err(|e| e.to_string())?;
    let Some(stored) = stored else { return Ok(None); };
    let preview: GapFollowupPreview = serde_json::from_str(&stored).map_err(|_| "followup_provenance_invalid")?;
    // A later task edit must not expand a source-bound draft. Checking only
    // that the original URL still exists would also admit additional targets.
    let scope_matches: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans s JOIN projects p ON p.id=s.project_id \
         WHERE s.id=?1 AND s.project_id=?2 AND p.status='active' \
         AND (SELECT COUNT(*) FROM sentinel_targets t WHERE t.scan_id=s.id)=1 \
         AND EXISTS(SELECT 1 FROM sentinel_targets t WHERE t.scan_id=s.id AND t.project_id=s.project_id AND t.url=?3))",
        params![scan_id, preview.project_id, preview.target_url], |r| r.get(0),
    ).map_err(|e| format!("followup_scope_lookup:{e}"))?;
    if !scope_matches { return Err("followup_source_scope_changed".into()); }
    let setup = authorization_control_setup_in(connection, app_data_dir, scan_id, &preview.target_url)?;
    if setup.contract_keys.is_empty() { return Err("followup_requires_explicit_authorization_controls".into()); }
    Ok(Some(setup.attempt_number))
}

fn gap_followup_relations(connection: &rusqlite::Transaction<'_>, scan_id: &str) -> Result<JsonValue, String> {
    let source: Option<String> = connection.query_row("SELECT source_preview_json FROM agent_gap_followups WHERE scan_id=?1",
        [scan_id], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
    let source = source.map(|text| serde_json::from_str::<GapFollowupPreview>(&text))
        .transpose().map_err(|_| "followup_provenance_invalid")?;
    let mut statement = connection.prepare(
        "SELECT f.scan_id,s.task_name,s.status,f.assessment_message_id FROM agent_gap_followups f \
         JOIN sentinel_scans s ON s.id=f.scan_id WHERE f.source_scan_id=?1 ORDER BY f.created_at,f.scan_id",
    ).map_err(|e| e.to_string())?;
    let mut tasks = statement.query_map([scan_id], |r| Ok(json!({"scanId":r.get::<_,String>(0)?,
        "taskName":r.get::<_,String>(1)?,"status":r.get::<_,String>(2)?,"assessmentMessageId":r.get::<_,String>(3)?})))
        .map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
    for task in &mut tasks {
        task["review"] = gap_review_status(connection, task["scanId"].as_str().ok_or("followup_scan_invalid")?)?;
    }
    let review = if source.is_some() { Some(gap_review_status(connection,scan_id)?) } else { None };
    // A source scan can have many unrelated gaps; never collapse one child's
    // receipt into a claim that every gap in the original scan is resolved.
    Ok(json!({"source":source,"tasks":tasks,"gapResolved":review.as_ref().is_some_and(|v|v["gapResolved"]==true),"review":review}))
}
