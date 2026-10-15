// Scanning keeps its frozen runtime snapshot; only connection material crosses
// the structured learning-model service boundary. No settings are re-resolved.
fn call_learning_llm(environment: &ModelRuntimeEnv, prompt: &str) -> Result<JsonValue, String> {
    crate::skills_catalog::learning_model::complete_with_connection(
        &environment.as_connection(),
        prompt,
    )
}

fn learning_candidate_source_hash(
    trace: &AgentTraceSummary,
    findings: &[(String, String)],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(trace.scan_id.as_bytes());
    hasher.update(trace.instruction_hash.as_bytes());
    hasher.update(trace.scan_type.as_bytes());
    for tool in &trace.tools {
        hasher.update(tool.name.as_bytes());
        hasher.update(tool.calls.to_le_bytes());
        hasher.update(tool.results.to_le_bytes());
    }
    for (title, severity) in findings {
        hasher.update(title.as_bytes());
        hasher.update(severity.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

#[derive(Debug, Clone)]
struct LearningQualityGate {
    disposition: &'static str,
    score: i64,
    evidence_count: usize,
    reusable_signal: bool,
    generic_only: bool,
    duplicate_tool_calls: usize,
    repeated_results: usize,
    cve_classes: HashMap<String, usize>,
    reasons: Vec<String>,
}

enum LearningGenerationOutcome {
    Candidate(AgentLearningCandidate),
    Skipped(LearningQualityGate),
}
