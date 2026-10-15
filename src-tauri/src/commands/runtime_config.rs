// Scanning policy is separate from the shared model connection. Preserve the
// historical serialized snapshot and all original resource-policy tuple fields.
#[derive(Clone, serde::Serialize)]
pub(crate) struct ModelRuntimeEnv {
    llm: String,
    api_key: String,
    api_base: String,
    deployment: String,
    full_power: bool,
    prompt_audit_mode: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LocalModelRuntimePolicy {
    parameter_billions: Option<u32>,
    unified_memory_gb: u64,
    max_concurrent_requests: usize,
    max_context_tokens: u64,
    max_output_tokens: Option<u64>,
    frontend_packet_budget_bytes: usize,
    memory_guard_tier: &'static str,
    startup_idle_seconds: u64,
    startup_hard_seconds: u64,
}

impl ModelRuntimeEnv {
    pub(crate) fn as_connection(&self) -> crate::model_configuration::ModelConnection {
        crate::model_configuration::ModelConnection {
            llm: self.llm.clone(),
            api_key: self.api_key.clone(),
            api_base: self.api_base.clone(),
            deployment: self.deployment.clone(),
        }
    }
}

fn model_runtime_env(settings: &JsonValue) -> Result<ModelRuntimeEnv, String> {
    let normalized = crate::model_configuration::NormalizedModelSettings::new(settings);
    let connection = crate::model_configuration::model_connection(&normalized)?;
    let settings = normalized.as_value();
    let deployment = connection.deployment;
    let full_power = deployment == "local"
        && settings
            .get("agentLocalFullPower")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
    let prompt_audit_mode = match settings
        .get("agentPromptAuditMode")
        .and_then(JsonValue::as_str)
        .unwrap_or("off")
    {
        "metadata" => "metadata",
        "full" => "full",
        _ => "off",
    }
    .to_string();
    Ok(ModelRuntimeEnv {
        llm: connection.llm,
        api_key: connection.api_key,
        api_base: connection.api_base,
        deployment,
        full_power,
        prompt_audit_mode,
    })
}

fn local_model_runtime_policy_for_memory(
    environment: &ModelRuntimeEnv,
    unified_memory_gb: u64,
) -> LocalModelRuntimePolicy {
    let policy = crate::model_configuration::model_resource_policy_for_memory(
        &environment.as_connection(),
        unified_memory_gb,
    );
    let (startup_idle_seconds, startup_hard_seconds) = if environment.deployment == "local" {
        match policy.parameter_billions {
            Some(size) if size >= 60 => (300, 1_800),
            Some(size) if size >= 20 => (240, 1_200),
            Some(size) if size >= 13 => (210, 1_200),
            Some(_) => (180, 900),
            None => (240, 1_200),
        }
    } else {
        (90, 300)
    };
    LocalModelRuntimePolicy {
        parameter_billions: policy.parameter_billions,
        unified_memory_gb: policy.unified_memory_gb,
        max_concurrent_requests: policy.max_concurrent_requests,
        max_context_tokens: policy.max_context_tokens,
        max_output_tokens: policy.max_output_tokens,
        memory_guard_tier: policy.memory_guard_tier,
        frontend_packet_budget_bytes: if environment.deployment != "local" {
            24 * 1024
        } else if unified_memory_gb <= 18 {
            6 * 1024
        } else if unified_memory_gb <= 32 {
            8 * 1024
        } else {
            12 * 1024
        },
        startup_idle_seconds,
        startup_hard_seconds,
    }
}

fn local_model_runtime_policy(environment: &ModelRuntimeEnv) -> LocalModelRuntimePolicy {
    local_model_runtime_policy_for_memory(
        environment,
        crate::model_configuration::system_unified_memory_gb(),
    )
}
fn agent_skill_instructions(
    connection: &rusqlite::Connection,
    skill_ids: &[i64],
) -> Result<(String, String), String> {
    if skill_ids.is_empty() {
        return Ok((String::new(), String::new()));
    }
    let ids = serde_json::to_string(skill_ids).map_err(|error| error.to_string())?;
    let sql = "SELECT name,instructions FROM agent_skills WHERE enabled=1 AND id IN (SELECT value FROM json_each(?1)) ORDER BY builtin DESC,id";
    let mut statement = connection.prepare(sql).map_err(|error| error.to_string())?;
    let rows: Vec<(String, String)> = statement
        .query_map([ids], agent_skill_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    const MAX_SKILL_CONTEXT_CHARS: usize = 48_000;
    const MAX_LARGE_SKILL_CHARS: usize = 24_000;

    let row_count = rows.len();
    let names = rows
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join("、");
    let mut used = 0usize;
    let mut rendered = Vec::new();
    for (name, body) in rows {
        if used >= MAX_SKILL_CONTEXT_CHARS {
            break;
        }
        let remaining = MAX_SKILL_CONTEXT_CHARS.saturating_sub(used);
        let per_skill_limit = if name.starts_with("sec_skills ·") {
            MAX_LARGE_SKILL_CHARS.min(remaining)
        } else {
            remaining
        };
        let compacted = compact_skill_context(&body, per_skill_limit);
        used = used.saturating_add(compacted.chars().count());
        rendered.push(format!("## {name}\n{compacted}"));
    }
    if rendered.len() < row_count {
        rendered.push(
            "\n[其余 Skill 已省略：达到单次任务 Skill 上下文上限；请在任务中显式选择所需 Skill。]"
                .into(),
        );
    }
    Ok((names, rendered.join("\n\n")))
}

/// Keep large imported method packs useful without copying their entire
/// archive into every model request. Section headings and a bounded excerpt
/// are retained so the Agent can recognize the relevant method family; the
/// full text remains editable in the local Skill card.
fn compact_skill_context(body: &str, max_chars: usize) -> String {
    if body.chars().count() <= max_chars {
        return body.to_string();
    }
    let mut output = String::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("##") {
            let heading = trimmed.to_string();
            if output.chars().count() + heading.chars().count() + 2 > max_chars {
                break;
            }
            output.push_str(&heading);
            output.push('\n');
            continue;
        }
        if output.chars().count() >= max_chars.saturating_sub(160) {
            break;
        }
        let remaining = max_chars.saturating_sub(output.chars().count() + 160);
        let excerpt = line.chars().take(remaining.min(720)).collect::<String>();
        if excerpt.trim().is_empty() {
            continue;
        }
        output.push_str(&excerpt);
        output.push('\n');
    }
    output.push_str(
        "\n[该 Skill 为本地大方法包；本次仅注入章节摘要，完整内容仍保存在 Oviraptor 本地。]",
    );
    output.chars().take(max_chars).collect()
}

fn agent_skill_row(row: &Row<'_>) -> rusqlite::Result<(String, String)> {
    Ok((row.get(0)?, row.get(1)?))
}

#[cfg(test)]
fn sentinel_process_set(
    db_path: &Path,
    scan_id: &str,
    process_id: u32,
    engine: &str,
    work_dir: &Path,
) {
    if let Ok(connection) = db::open(db_path) {
        let _ = connection.execute(
            "INSERT INTO sentinel_processes(scan_id,process_id,engine,work_dir) VALUES(?1,?2,?3,?4) ON CONFLICT(scan_id,process_id) DO UPDATE SET engine=excluded.engine,work_dir=excluded.work_dir,started_at=datetime('now','localtime')",
            params![scan_id, process_id as i64, engine, work_dir.to_string_lossy()],
        );
    }
}

#[cfg(test)]
fn sentinel_process_clear(db_path: &Path, scan_id: &str, process_id: u32) {
    if let Ok(connection) = db::open(db_path) {
        let _ = connection.execute(
            "DELETE FROM sentinel_processes WHERE scan_id=?1 AND process_id=?2",
            params![scan_id, process_id as i64],
        );
    }
}

fn sentinel_scan_update(db_path: &Path, scan_id: &str, status: &str, checkpoint: &str) {
    if let Ok(connection) = db::open(db_path) {
        let deleted = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_deleted_scans WHERE scan_id=?1",
                [scan_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0);
        if deleted == 0 {
            let changed = connection.execute(
                "UPDATE sentinel_scans SET status=?1,current_checkpoint=?2,updated_at=datetime('now','localtime') WHERE id=?3 AND status NOT IN ('pausing','paused') AND NOT EXISTS(SELECT 1 FROM native_web_attempt_closures c WHERE c.scan_id=sentinel_scans.id AND c.attempt_number=sentinel_scans.attempt_count)",
                params![status, checkpoint, scan_id],
            );
            if matches!(changed,Ok(1)) { sync_sentinel_attempt(&connection, scan_id); }
        }
    }
}

fn sentinel_scan_is_active(db_path: &Path, scan_id: &str) -> bool {
    let Ok(connection) = db::open(db_path) else {
        return false;
    };
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND status IN ('scanning','pausing')) AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
            [scan_id],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false)
}

fn sentinel_scan_pause_requested(db_path: &Path, scan_id: &str) -> bool {
    let Ok(connection) = db::open(db_path) else {
        return false;
    };
    connection
        .query_row(
            "SELECT status='pausing' FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false)
}

#[cfg(test)]
fn agent_batch_size(settings: &JsonValue) -> usize {
    db::normalize_settings(settings)
        .get("agentBatchSize")
        .and_then(JsonValue::as_u64)
        .unwrap_or(15)
        .clamp(1, 50) as usize
}

fn setting_u64(
    settings: &JsonValue,
    key: &str,
    fallback: u64,
    min: u64,
    max: u64,
) -> u64 {
    settings
        .get(key)
        .and_then(JsonValue::as_u64)
        .unwrap_or(fallback)
        .clamp(min, max)
}

/// The configurable uncached-token budget uses zero to disable that one layer.
/// Absolute cumulative-context and loop fuses are applied per target later.
fn token_limit(
    settings: &JsonValue,
    key: &str,
    fallback: u64,
    min: u64,
    max: u64,
) -> i64 {
    let Some(value) = settings.get(key) else {
        return fallback as i64;
    };
    if value.as_i64() == Some(0) || value.as_u64() == Some(0) {
        return 0;
    }
    value
        .as_u64()
        .map(|number| number.clamp(min, max) as i64)
        .unwrap_or(fallback as i64)
}

#[derive(Clone, Debug, serde::Serialize)]
struct AgentBudgetSettings {
    quick_score: i64,
    standard_score: i64,
    deep_score: i64,
    quick_timeout: u64,
    standard_timeout: u64,
    deep_timeout: u64,
    quick_tokens: i64,
    standard_tokens: i64,
    deep_tokens: i64,
    quick_requests: i64,
    standard_requests: i64,
    deep_requests: i64,
    no_tool_turn_limit: i64,
    max_mode: String,
    max_budget_usd: Option<f64>,
}

impl AgentBudgetSettings {
    fn from_json(settings: &JsonValue) -> Self {
        let normalized = db::normalize_settings(settings);
        let settings = &normalized;
        let quick_score =
            setting_u64(settings, "agentQuickScore", 30, 1, 90) as i64;
        let standard_score = setting_u64(
            settings,
            "agentStandardScore",
            55,
            (quick_score + 1) as u64,
            95,
        ) as i64;
        let deep_score = setting_u64(
            settings,
            "agentDeepScore",
            80,
            (standard_score + 1) as u64,
            100,
        ) as i64;
        Self {
            quick_score,
            standard_score,
            deep_score,
            quick_timeout: setting_u64(
                settings,
                "agentQuickTimeout",
                240,
                30,
                3600,
            ),
            standard_timeout: setting_u64(
                settings,
                "agentStandardTimeout",
                600,
                60,
                7200,
            ),
            deep_timeout: setting_u64(
                settings,
                "agentDeepTimeout",
                1_200,
                120,
                14400,
            ),
            quick_tokens: token_limit(
                settings,
                "agentQuickTokenLimit",
                200_000,
                10_000,
                20_000_000,
            ),
            standard_tokens: token_limit(
                settings,
                "agentStandardTokenLimit",
                400_000,
                20_000,
                40_000_000,
            ),
            deep_tokens: token_limit(
                settings,
                "agentDeepTokenLimit",
                800_000,
                50_000,
                80_000_000,
            ),
            quick_requests: setting_u64(
                settings,
                "agentQuickRequestLimit",
                6,
                1,
                100,
            ) as i64,
            standard_requests: setting_u64(
                settings,
                "agentStandardRequestLimit",
                14,
                1,
                200,
            ) as i64,
            deep_requests: setting_u64(
                settings,
                "agentDeepRequestLimit",
                24,
                1,
                300,
            ) as i64,
            no_tool_turn_limit: setting_u64(
                settings,
                "agentNoToolTurnLimit",
                6,
                1,
                100,
            ) as i64,
            max_mode: "deep".into(),
            max_budget_usd: None,
        }
    }

    fn apply_deployment(&mut self, deployment: &str) {
        if deployment != "local" {
            return;
        }
        // Local models still get bounded work, but the old 50k/120s cap was
        // too small for authenticated frontend evidence and caused partial
        // scans after browser capture had already succeeded. Keep an explicit
        // ceiling without silently collapsing the configured budget.
        self.quick_timeout = self.quick_timeout.min(300);
        self.standard_timeout = self.standard_timeout.min(480);
        self.deep_timeout = self.deep_timeout.min(900);
        if self.quick_tokens > 0 {
            self.quick_tokens = self.quick_tokens.min(200_000);
        }
        if self.standard_tokens > 0 {
            self.standard_tokens = self.standard_tokens.min(400_000);
        }
        if self.deep_tokens > 0 {
            self.deep_tokens = self.deep_tokens.min(700_000);
        }
        self.quick_requests = self.quick_requests.min(8);
        self.standard_requests = self.standard_requests.min(12);
        self.deep_requests = self.deep_requests.min(16);
        self.no_tool_turn_limit = self.no_tool_turn_limit.min(6);
    }

    fn apply_web_policy(&mut self, policy: &JsonValue) {
        self.max_mode = match policy.get("webModeCeiling").and_then(JsonValue::as_str) {
            Some("quick") => "quick",
            Some("deep") => "deep",
            _ => "standard",
        }
        .into();
        self.max_budget_usd = policy
            .get("maxBudgetUsd")
            .and_then(JsonValue::as_f64)
            .filter(|value| *value > 0.0 && *value <= 10_000.0);
    }

    fn bounded_mode(&self, mode: &str) -> String {
        let rank = |value: &str| match value {
            "deep" => 3,
            "standard" => 2,
            "quick" => 1,
            _ => 0,
        };
        if rank(mode) > rank(&self.max_mode) {
            self.max_mode.clone()
        } else {
            mode.to_string()
        }
    }

    fn mode_for_score(&self, score: i64) -> &'static str {
        if score < self.quick_score {
            "skip"
        } else if score < self.standard_score {
            "quick"
        } else if score < self.deep_score {
            "standard"
        } else {
            "deep"
        }
    }

    fn limits(&self, mode: &str) -> (u64, i64, i64) {
        match mode {
            "deep" => (self.deep_timeout, self.deep_tokens, self.deep_requests),
            "standard" => (
                self.standard_timeout,
                self.standard_tokens,
                self.standard_requests,
            ),
            _ => (self.quick_timeout, self.quick_tokens, self.quick_requests),
        }
    }
}
