#[tauri::command]
pub fn analyze_agent_trace(
    state: State<AppState>,
    scan_id: String,
) -> Result<AgentKnowledgeEntry, String> {
    let connection = db::open(&state.db_path)?;
    let (trace, _) = collect_agent_trace(&connection, &scan_id, false, true)?;
    let project_id = connection
        .query_row(
            "SELECT project_id FROM sentinel_scans WHERE id=?1",
            [&scan_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .unwrap_or(None);
    let mut statement = connection
        .prepare("SELECT DISTINCT title,severity FROM sentinel_findings WHERE scan_id=?1 AND (kind LIKE '%vulnerab%' OR kind='risk') AND trim(title)<>'' ORDER BY severity,title LIMIT 20")
        .map_err(|error| error.to_string())?;
    let findings = statement
        .query_map([&scan_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?
        .flatten()
        .collect::<Vec<_>>();
    drop(statement);
    let top_tools = trace
        .tools
        .iter()
        .take(12)
        .map(|tool| tool.name.clone())
        .collect::<Vec<_>>();
    let finding_names = findings
        .iter()
        .map(|item| item.0.clone())
        .collect::<Vec<_>>();
    let mut canonical_hasher = Sha256::new();
    canonical_hasher.update(b"task-knowledge-v2");
    canonical_hasher.update(trace.scan_type.as_bytes());
    for tool in &top_tools {
        canonical_hasher.update(tool.to_ascii_lowercase().as_bytes());
    }
    for finding in &finding_names {
        canonical_hasher.update(canonical_learning_text(finding).as_bytes());
    }
    let canonical_key = format!("{:x}", canonical_hasher.finalize());
    let quality_score = trace_quality_score(&trace, findings.len());
    if quality_score < 60 {
        return Err(format!(
            "该任务轨迹质量仅 {quality_score}/100：至少需要完整运行产物、Agent 消息和工具调用/结果闭环；未写入知识库"
        ));
    }
    let title = if trace.task_name.trim().is_empty() {
        format!("{} · {} 轨迹知识", trace.project_name, trace.scan_type)
    } else {
        format!("{} · 轨迹知识", trace.task_name)
    };
    let summary = format!(
        "候选知识质量 {}/100：本地分析 {} 个运行、{} 个 Agent、{} 条消息和 {} 次工具调用；识别 {} 类已入库安全问题。该知识不包含目标凭据或原始工具参数。",
        quality_score, trace.run_count, trace.agent_count, trace.message_count, trace.tool_call_count, findings.len()
    );
    let patterns = serde_json::json!({
        "schemaVersion": 2,
        "normalizerVersion": "learning-canonical-v2",
        "knowledgeKind": "task_candidate",
        "sourceAuthority": trace.source_authority,
        "canonicalKey": canonical_key,
        "qualityScore": quality_score,
        "scanType": trace.scan_type,
        "model": trace.model,
        "producer": {"model":trace.model,"instructionHash":trace.instruction_hash},
        "support": {"distinctScans":1,"distinctModels":1,"eligibleForAutomaticMerge":false},
        "facts": {"tools":top_tools,"findingClasses":finding_names},
        "tools": top_tools,
        "findingClasses": finding_names,
        "messageCount": trace.message_count,
        "reasoningCount": trace.reasoning_count,
        "toolCalls": trace.tool_call_count,
        "toolResults": trace.tool_result_count,
        "llmRequests": trace.llm_requests,
        "hookedRequests": trace.hooked_request_count,
        "requestUsageEntries": trace.usage_entry_count,
        "usageAgents": trace.usage_agent_count,
        "tokenUsageEstimated": trace.token_usage_estimated,
        "instructionHash": trace.instruction_hash,
        "learningPolicy": {"factsAreDeterministic":true,"modelTextIsProposal":true,"sameScanDifferentModelAddsSupport":false},
    });
    let finding_lines = if findings.is_empty() {
        "- 从资产证据和数据流开始，仅保留可复现且有安全影响的问题。".to_string()
    } else {
        findings
            .iter()
            .take(12)
            .map(|(name, severity)| {
                format!("- 验证 {name}（历史严重度：{severity}），不要仅凭指纹或路径推断。")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let tool_lines = if trace.tools.is_empty() {
        "- 先读取确定性证据，再选择最小必要验证工具。".to_string()
    } else {
        trace
            .tools
            .iter()
            .take(10)
            .map(|tool| {
                format!(
                    "- `{}`：历史调用 {} 次；仅在有新证据时继续。",
                    tool.name, tool.calls
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let skill_instructions = format!(
        "## Objective\n复用已验证的分析路径，提高同类任务的证据质量和停止判断。\n\n## Proven tool workflow\n{tool_lines}\n\n## Vulnerability focus\n{finding_lines}\n\n## Guardrails\n- 不复制历史目标、Cookie、Token、请求头或原始敏感参数。\n- 每个结论必须绑定新任务中的 URL、代码位置或请求响应证据。\n- 两次验证没有新增证据时切换候选；不要把侦察信息升级为漏洞。\n- 保留 CVSS、CWE、PoC 和修复建议结构。"
    );
    let mut source_hasher = Sha256::new();
    source_hasher.update(b"task-knowledge-source-v2");
    source_hasher.update(canonical_key.as_bytes());
    source_hasher.update(trace.instruction_hash.as_bytes());
    let source_hash = format!("{:x}", source_hasher.finalize());
    connection.execute(
        "INSERT INTO agent_knowledge_entries(scan_id,project_id,title,summary,patterns_json,skill_instructions,source_hash) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(scan_id) DO UPDATE SET project_id=excluded.project_id,title=excluded.title,summary=excluded.summary,patterns_json=excluded.patterns_json,skill_instructions=excluded.skill_instructions,source_hash=excluded.source_hash,updated_at=datetime('now','localtime')",
        params![scan_id,project_id,title,summary,patterns.to_string(),skill_instructions,source_hash],
    ).map_err(|error| error.to_string())?;
    crate::skills_catalog::knowledge::knowledge_for_source(&connection, &scan_id)
        .map_err(|error| error.to_string())
}

fn recurring_workflow_signals(events: &[AgentTraceEvent]) -> HashSet<String> {
    const NOISE_TOOLS: &[&str] = &[
        "create_todo",
        "update_todo",
        "create_note",
        "write_stdin",
        "finish_scan",
        "wait",
        "view_image",
        "create_agent",
        "stop_agent",
        "wait_for_message",
        "view_agent_graph",
        "web_search",
        "mark_todo_done",
        "delete_todo",
    ];
    const COMMAND_TOOLS: &[&str] = &[
        "curl",
        "httpx",
        "nuclei",
        "semgrep",
        "codeql",
        "ffuf",
        "sqlmap",
        "nikto",
        "nmap",
        "subfinder",
        "katana",
        "playwright",
        "python",
        "node",
    ];
    let mut signals = HashSet::new();
    for event in events
        .iter()
        .filter(|event| event.event_type == "function_call")
    {
        if event.name == "exec_command" {
            let lower = event.detail.to_ascii_lowercase();
            for tool in COMMAND_TOOLS {
                if lower.contains(tool) {
                    signals.insert(format!("exec:{tool}"));
                }
            }
        } else if !event.name.is_empty() && !NOISE_TOOLS.contains(&event.name.as_str()) {
            signals.insert(event.name.clone());
        }
    }
    signals
}

#[tauri::command]
pub fn aggregate_agent_knowledge(
    state: State<AppState>,
    scan_type: String,
) -> Result<AgentKnowledgeEntry, String> {
    let scan_type = scan_type.trim().to_ascii_lowercase();
    if !["web", "code", "greybox", "cicd"].contains(&scan_type.as_str()) {
        return Err("不支持的任务类型".into());
    }
    let connection = db::open(&state.db_path)?;
    let mut statement = connection
        .prepare("SELECT id FROM sentinel_scans WHERE scan_type=?1 AND task_path<>'' ORDER BY created_at DESC LIMIT 80")
        .map_err(|error| error.to_string())?;
    let scan_ids = statement
        .query_map([&scan_type], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .flatten()
        .collect::<Vec<_>>();
    drop(statement);

    let mut qualified_ids = Vec::new();
    let mut excluded = 0i64;
    let mut workflow_support: HashMap<String, i64> = HashMap::new();
    let mut finding_support: HashMap<String, (String, String, i64)> = HashMap::new();
    let mut source_models = HashSet::new();
    let mut source_hasher = Sha256::new();
    for scan_id in scan_ids {
        let Ok((trace, events)) = collect_agent_trace(&connection, &scan_id, true, true) else {
            excluded += 1;
            continue;
        };
        let mut finding_statement = connection
            .prepare("SELECT DISTINCT title,severity FROM sentinel_findings WHERE scan_id=?1 AND (kind LIKE '%vulnerab%' OR kind='risk') AND trim(title)<>'' ORDER BY severity,title LIMIT 50")
            .map_err(|error| error.to_string())?;
        let findings = finding_statement
            .query_map([&scan_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?
            .flatten()
            .collect::<Vec<_>>();
        drop(finding_statement);
        if trace_quality_score(&trace, findings.len()) < 60 {
            excluded += 1;
            continue;
        }
        qualified_ids.push(scan_id.clone());
        if !trace.model.trim().is_empty() {
            source_models.insert(trace.model.clone());
        }
        source_hasher.update(scan_id.as_bytes());
        source_hasher.update(trace.instruction_hash.as_bytes());
        for signal in recurring_workflow_signals(&events) {
            *workflow_support.entry(signal).or_default() += 1;
        }
        let mut scan_findings = HashSet::new();
        for (title, severity) in findings {
            let normalized = title.trim().to_ascii_lowercase();
            if normalized.is_empty() || !scan_findings.insert(normalized.clone()) {
                continue;
            }
            let entry = finding_support
                .entry(normalized)
                .or_insert((title, severity, 0));
            entry.2 += 1;
        }
    }
    let qualified = qualified_ids.len() as i64;
    if qualified < 2 {
        return Err(format!(
            "同类型轨迹中只有 {qualified} 个通过质量门槛；至少需要 2 个包含 Agent、工具调用和结果闭环的任务"
        ));
    }
    let mut workflows = workflow_support
        .into_iter()
        .filter(|(_, support)| *support >= 2 && *support * 2 >= qualified)
        .collect::<Vec<_>>();
    workflows.sort_by_key(|(_, support)| std::cmp::Reverse(*support));
    let mut findings = finding_support
        .into_values()
        .filter(|(_, _, support)| *support >= 2 && *support * 2 >= qualified)
        .collect::<Vec<_>>();
    findings.sort_by_key(|(_, _, support)| std::cmp::Reverse(*support));
    if workflows.is_empty() && findings.is_empty() {
        return Err(
            "轨迹数量足够，但没有形成跨任务重复出现的工具路径或漏洞类别；未生成噪音知识".into(),
        );
    }

    let confidence = (45
        + qualified.min(5) * 5
        + workflows.len().min(6) as i64 * 2
        + findings.len().min(4) as i64 * 4)
        .min(92);
    let scan_label = match scan_type.as_str() {
        "web" => "Web URL",
        "code" => "代码审计",
        "greybox" => "灰盒联测",
        "cicd" => "CI/CD",
        _ => &scan_type,
    };
    let workflow_json = workflows
        .iter()
        .map(|(name, support)| serde_json::json!({"name":name,"support":support,"total":qualified}))
        .collect::<Vec<_>>();
    let finding_json = findings
        .iter()
        .map(|(title, severity, support)| serde_json::json!({"title":title,"severity":severity,"support":support,"total":qualified}))
        .collect::<Vec<_>>();
    let tool_names = workflows
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    let finding_names = findings
        .iter()
        .map(|(title, _, _)| title.clone())
        .collect::<Vec<_>>();
    let mut source_models = source_models.into_iter().collect::<Vec<_>>();
    source_models.sort();
    let mut canonical_hasher = Sha256::new();
    canonical_hasher.update(b"aggregate-knowledge-v2");
    canonical_hasher.update(scan_type.as_bytes());
    for name in &tool_names {
        canonical_hasher.update(name.to_ascii_lowercase().as_bytes());
    }
    for name in &finding_names {
        canonical_hasher.update(canonical_learning_text(name).as_bytes());
    }
    let canonical_key = format!("{:x}", canonical_hasher.finalize());
    let patterns = serde_json::json!({
        "schemaVersion": 2,
        "normalizerVersion": "learning-canonical-v2",
        "knowledgeKind": "aggregate",
        "canonicalKey": canonical_key,
        "scanType": scan_type,
        "qualityScore": confidence,
        "sourceScans": qualified,
        "excludedScans": excluded,
        "sourceScanIds": qualified_ids,
        "sourceModels": source_models,
        "support": {"distinctScans":qualified,"distinctModels":source_models.len(),"eligibleForAutomaticMerge":true},
        "tools": tool_names,
        "findingClasses": finding_names,
        "recurringWorkflow": workflow_json,
        "recurringFindings": finding_json,
        "qualityGate": "run + agent messages + >=2 tool calls + >=50% tool result closure; pattern support >=2 and >=50%",
        "learningPolicy": {"sameScanDifferentModelAddsSupport":false,"factsAreDeterministic":true,"modelWordingExcludedFromCanonicalKey":true},
    });
    source_hasher.update(patterns.to_string().as_bytes());
    let source_hash = format!("{:x}", source_hasher.finalize());
    let title = format!("{scan_label} · 多任务聚合知识");
    let summary = format!(
        "聚合 {qualified} 个高质量同类任务，排除 {excluded} 个不完整或低信号任务；保留 {} 条重复工作流和 {} 类重复漏洞，可信度 {confidence}/100。单次目标、凭据、原始命令参数和偶发工具噪音均未进入知识。",
        workflows.len(), findings.len()
    );
    let workflow_lines = if workflows.is_empty() {
        "- 没有达到跨任务支持门槛的工具路径；从新任务的确定性证据开始。".to_string()
    } else {
        workflows
            .iter()
            .map(|(name, support)| format!("- `{name}`：在 {support}/{qualified} 个高质量任务中出现；仅在同类证据成立时复用。"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let finding_lines = if findings.is_empty() {
        "- 不预设漏洞类型；只接受新任务中可复现的安全影响。".to_string()
    } else {
        findings
            .iter()
            .map(|(title, severity, support)| format!("- {title}（历史 {severity}，支持 {support}/{qualified}）：必须重新验证输入、影响与边界。"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let skill_instructions = format!(
        "## Aggregated scope\n来自 {qualified} 个通过质量门槛的 {scan_label} 任务；可信度 {confidence}/100。只复用跨任务重复模式，不复用目标数据。\n\n## Recurring workflow\n{workflow_lines}\n\n## Recurring vulnerability focus\n{finding_lines}\n\n## Guardrails\n- 新任务必须重新建立 URL、参数、代码位置、请求响应或数据流证据。\n- 不携带历史域名、IP、Cookie、Token、请求头、凭据或原始命令。\n- 单次偶发现象、纯侦察输出、todo/备注/等待操作不得升级为漏洞。\n- 两次验证没有新增证据时停止当前分支，并记录停止原因。"
    );
    let aggregate_id = format!("aggregate:{scan_type}");
    connection.execute(
        "INSERT INTO agent_knowledge_entries(scan_id,title,summary,patterns_json,skill_instructions,source_hash) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(scan_id) DO UPDATE SET title=excluded.title,summary=excluded.summary,patterns_json=excluded.patterns_json,skill_instructions=excluded.skill_instructions,source_hash=excluded.source_hash,updated_at=datetime('now','localtime')",
        params![aggregate_id,title,summary,patterns.to_string(),skill_instructions,source_hash],
    ).map_err(|error| error.to_string())?;
    crate::skills_catalog::knowledge::knowledge_for_source(&connection, &aggregate_id)
        .map_err(|error| error.to_string())
}
