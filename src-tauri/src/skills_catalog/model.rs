//! Skills Catalog transport contracts. Compatibility exports live in `crate::models`.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub builtin: bool,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkillInput {
    pub id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub instructions: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentKnowledgeEntry {
    pub id: i64,
    pub scan_id: String,
    pub project_id: Option<i64>,
    pub title: String,
    pub summary: String,
    pub patterns: Value,
    pub source_hash: String,
    pub skill_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AgentLearningCandidate {
    pub id: i64,
    pub scan_id: String,
    pub project_id: Option<i64>,
    pub scan_type: String,
    pub title: String,
    pub summary: String,
    pub candidate: Value,
    pub status: String,
    pub target_skill_id: Option<i64>,
    pub source_hash: String,
    pub created_at: String,
    pub reviewed_at: String,
    pub updated_at: String,
}

fn default_true() -> bool {
    true
}
