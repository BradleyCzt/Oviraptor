fn generate_learning_candidate_with_environment(
    db_path: &Path,
    scan_id: &str,
    environment: &ModelRuntimeEnv,
) -> Result<LearningGenerationOutcome, String> {
    let connection = db::open(db_path)?;
    let (trace, events) = collect_agent_trace(&connection, scan_id, true, true)?;
    if !["completed", "partial", "failed"].contains(&trace.status.as_str()) {
        return Err("扫描尚未结束，暂不生成学习候选".into());
    }
    let mut statement = connection
        .prepare("SELECT DISTINCT title,severity,kind,record_json FROM sentinel_findings WHERE scan_id=?1 AND trim(title)<>'' ORDER BY severity,title LIMIT 40")
        .map_err(|error| error.to_string())?;
    let findings = statement
        .query_map([scan_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .flatten()
        .collect::<Vec<_>>();
    drop(statement);
    let finding_pairs = findings
        .iter()
        .map(|(title, severity, _, _)| (title.clone(), severity.clone()))
        .collect::<Vec<_>>();
    let source_hash = learning_candidate_source_hash(&trace, &finding_pairs);
    let existing =
        crate::skills_catalog::knowledge::candidate_for_source(&connection, scan_id, &source_hash)
            .optional()
            .map_err(|error| error.to_string())?;
    if let Some(existing) =
        existing.filter(|candidate| matches!(candidate.status.as_str(), "accepted" | "applied"))
    {
        return Ok(LearningGenerationOutcome::Candidate(existing));
    }
    let event_digest = events
        .iter()
        .filter(|event| {
            matches!(
                event.event_type.as_str(),
                "function_call" | "function_call_output" | "message"
            )
        })
        .take(80)
        .map(|event| {
            format!(
                "{} {} {}",
                event.event_type,
                event.name,
                retained_trace_text(&event.detail)
                    .chars()
                    .take(800)
                    .collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let quality_gate = assess_learning_quality(&trace, &events, &findings);
    // Learning is a post-scan optimization, not part of scan success. If the
    // deterministic gate already knows there is no reusable evidence, do not
    // spend another model call asking an LLM to reach the same conclusion.
    if quality_gate.disposition != "reusable_candidate" {
        return Ok(LearningGenerationOutcome::Skipped(quality_gate));
    }
    let cached_knowledge = cached_external_knowledge_context(&connection);
    let findings_json = findings
        .iter()
        .map(|(title, severity, kind, record_json)| {
            serde_json::json!({
                "title":title,
                "severity":severity,
                "kind":kind,
                "classification":classify_finding_signal(title, kind, record_json),
                "hasEvidence":finding_has_evidence(record_json),
            })
        })
        .collect::<Vec<_>>();
    let prompt = format!(
        "请分析以下一次 {scan_type} 扫描，并输出 JSON：\n\n扫描概要：{trace_json}\n\n安全发现：{findings}\n\n调用摘要：{events}\n\n本地质量预检：{quality}\n\n已缓存的公开来源方法卡片（仅作思路索引，不等于本次证据）：\n{cached_knowledge}\n\n输出字段必须包含 summary、newIdeas（数组，每项含 title/problem/evidence/confidence/action）、redundantSteps、weakSteps、externalKnowledgeRequests、skillPatch、qualityGate。skillPatch 使用 addSections/replaceSections/removeSections/keepSections/reasoning；只有可跨目标复用、证据充分且不会引入误报的内容才放入 addSections。若只是普通探测、版本匹配、无复现 CVE 或没有新增证据，qualityGate.disposition 必须为 no_learning_value 或 needs_verification，并将 skillPatch 留空。CVE 必须区分 confirmed、needs_verification、dependency_signal、info、invalid_or_duplicate，禁止把 NVD/版本匹配直接写成已确认漏洞。",
        scan_type = trace.scan_type,
        trace_json = serde_json::to_string(&serde_json::json!({
            "taskName":trace.task_name,"project":trace.project_name,"status":trace.status,"model":trace.model,
            "sourceAuthority":trace.source_authority,
            "runCount":trace.run_count,"agentCount":trace.agent_count,"messageCount":trace.message_count,
            "toolCalls":trace.tool_call_count,"toolResults":trace.tool_result_count,"tokens":trace.total_tokens,
            "tools":trace.tools.iter().map(|tool| serde_json::json!({"name":tool.name,"calls":tool.calls,"results":tool.results})).collect::<Vec<_>>()
        })).unwrap_or_default(),
        findings = serde_json::to_string(&findings_json).unwrap_or_default(),
        events = event_digest,
        cached_knowledge = cached_knowledge,
        quality = serde_json::json!({
            "disposition":quality_gate.disposition,
            "score":quality_gate.score,
            "evidenceCount":quality_gate.evidence_count,
            "reusableSignal":quality_gate.reusable_signal,
            "genericOnly":quality_gate.generic_only,
            "duplicateToolCalls":quality_gate.duplicate_tool_calls,
            "repeatedResults":quality_gate.repeated_results,
            "findingClasses":quality_gate.cve_classes,
            "reasons":quality_gate.reasons,
        }),
    );
    let mut candidate_json = call_learning_llm(environment, &prompt)
        .unwrap_or_else(|_| fallback_learning_candidate(&trace, &finding_pairs));
    enforce_learning_quality(&mut candidate_json, &quality_gate);
    canonicalize_learning_candidate(&mut candidate_json, &trace, &findings, environment, &prompt);
    let title = candidate_json
        .get("title")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            if trace.task_name.trim().is_empty() {
                "扫描后的学习候选"
            } else {
                trace.task_name.as_str()
            }
        })
        .chars()
        .take(120)
        .collect::<String>();
    let summary = candidate_json
        .get("summary")
        .and_then(JsonValue::as_str)
        .unwrap_or("模型未提供摘要；请查看候选详情")
        .chars()
        .take(2000)
        .collect::<String>();
    let project_id = connection
        .query_row(
            "SELECT project_id FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .unwrap_or(None);
    connection
        .execute(
            "INSERT INTO agent_learning_candidates(scan_id,project_id,scan_type,title,summary,candidate_json,status,source_hash) VALUES(?1,?2,?3,?4,?5,?6,'pending',?7) ON CONFLICT(scan_id,source_hash) DO UPDATE SET title=excluded.title,summary=excluded.summary,candidate_json=excluded.candidate_json,status='pending',reviewed_at='',updated_at=datetime('now','localtime') WHERE agent_learning_candidates.status IN ('pending','rejected')",
        params![scan_id, project_id, trace.scan_type, title, summary, candidate_json.to_string(), source_hash],
        )
        .map_err(|error| error.to_string())?;
    let candidate =
        crate::skills_catalog::knowledge::candidate_for_source(&connection, scan_id, &source_hash)
            .map_err(|error| error.to_string())?;
    Ok(LearningGenerationOutcome::Candidate(candidate))
}

fn scan_supports_automatic_learning(db_path: &Path, scan_id: &str) -> bool {
    db::open(db_path)
        .ok()
        .and_then(|connection| {
            connection
                .query_row(
                    "SELECT status IN ('completed','partial') FROM sentinel_scans WHERE id=?1",
                    [scan_id],
                    |row| row.get::<_, bool>(0),
                )
                .ok()
        })
        .unwrap_or(false)
}

fn schedule_learning_candidate(db_path: PathBuf, scan_id: String, environment: ModelRuntimeEnv) {
    thread::spawn(move || {
        let result = generate_learning_candidate_with_environment(&db_path, &scan_id, &environment);
        if let Ok(connection) = db::open(&db_path) {
            let outcome = match result {
                Ok(LearningGenerationOutcome::Candidate(candidate)) => {
                    let disposition = candidate
                        .candidate
                        .pointer("/qualityGate/disposition")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("unknown");
                    let idea_count = candidate
                        .candidate
                        .get("newIdeas")
                        .and_then(JsonValue::as_array)
                        .map(|items| items.len())
                        .unwrap_or(0);
                    serde_json::json!({
                        "status":"candidate_ready",
                        "disposition":disposition,
                        "candidateId":candidate.id,
                        "ideaCount":idea_count,
                        "summary":format!("已生成 {idea_count} 条通过质量预检的待审核学习候选"),
                    })
                }
                Ok(LearningGenerationOutcome::Skipped(gate)) => serde_json::json!({
                    "status":"skipped",
                    "disposition":gate.disposition,
                    "score":gate.score,
                    "evidenceCount":gate.evidence_count,
                    "reasons":gate.reasons,
                    "summary":"本次扫描没有形成可跨目标复用的证据链，已跳过学习模型调用；扫描结果不受影响",
                }),
                Err(error) => serde_json::json!({
                    "status":"error",
                    "summary":"扫描结果已保留，但后台学习分析未完成",
                    "error":error,
                }),
            };
            // Keep the terminal scan summary intact. Learning has its own
            // checkpoint and can never turn a completed scan into a perceived
            // failure or hide the original failure reason.
            let _ = connection.execute(
                "INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES(?1,'*','learning_outcome',?2) ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=datetime('now','localtime')",
                params![scan_id, outcome.to_string()],
            );
        }
    });
}

#[tauri::command]
pub fn generate_agent_learning_candidate(
    state: State<AppState>,
    scan_id: String,
) -> Result<AgentLearningCandidate, String> {
    let settings = sentinel_settings(&db::open(&state.db_path)?);
    let environment = model_runtime_env(&settings)?;
    match generate_learning_candidate_with_environment(&state.db_path, &scan_id, &environment)? {
        LearningGenerationOutcome::Candidate(candidate) => Ok(candidate),
        LearningGenerationOutcome::Skipped(gate) => Err(format!(
            "本次扫描没有可沉淀的学习候选（{}，{}/100）：{}。扫描结果本身不受影响，也没有消耗额外的学习模型调用",
            gate.disposition,
            gate.score,
            if gate.reasons.is_empty() {
                "没有形成可跨目标复用的证据链".to_string()
            } else {
                gate.reasons.join("；")
            }
        )),
    }
}

fn trace_quality_score(trace: &AgentTraceSummary, finding_count: usize) -> i64 {
    let mut score = 0;
    if trace.run_count > 0 {
        score += 25;
    }
    if trace.agent_count > 0 && trace.message_count >= 3 {
        score += 20;
    }
    if trace.tool_call_count >= 2 && trace.tool_result_count * 2 >= trace.tool_call_count.max(1) {
        score += 25;
    }
    if trace.reasoning_count > 0 {
        score += 10;
    }
    if finding_count > 0 {
        score += 20;
    }
    score
}
