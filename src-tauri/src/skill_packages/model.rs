use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSummary {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub entry_path: String,
    pub enabled: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageInfo {
    pub id: String,
    pub name: String,
    pub source: String,
    pub version: String,
    pub license: String,
    pub builtin: bool,
    pub digest: String,
    pub skill_count: usize,
    pub file_count: usize,
    pub total_bytes: usize,
    pub created_at: String,
    pub skills: Vec<SkillSummary>,
    pub missing_capabilities: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSkill {
    pub name: String,
    pub description: String,
    pub entry_path: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackagePreview {
    pub digest: String,
    pub name: String,
    pub skill_count: usize,
    pub file_count: usize,
    pub total_bytes: usize,
    pub skills: Vec<PreviewSkill>,
    pub missing_capabilities: Vec<String>,
    pub warnings: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportInput {
    pub path: String,
    pub expected_digest: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Manifest {
    pub schema_version: i64,
    pub package_key: String,
    pub name: String,
    pub version: String,
    pub source: String,
    pub license: String,
    pub files: Vec<ManifestFile>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ManifestFile {
    pub path: String,
    pub sha256: String,
    pub content_base64: String,
}
pub(super) struct PackageFile {
    pub path: String,
    pub bytes: Vec<u8>,
    pub sha256: String,
}
pub(super) struct PackageSkill {
    pub name: String,
    pub description: String,
    pub entry_path: String,
    pub instructions: String,
}
pub(super) struct Package {
    pub id: String,
    pub package_key: String,
    pub name: String,
    pub version: String,
    pub source: String,
    pub license: String,
    pub builtin: bool,
    pub digest: String,
    pub files: Vec<PackageFile>,
    pub skills: Vec<PackageSkill>,
    pub total_bytes: usize,
}
pub(super) fn capability_notices() -> Vec<String> {
    vec!["Packages provide instructions and reference files only. Imported scripts are inert; shell execution, browser automation, external integrations, and other tools mentioned by a skill are not installed or authorized by this package.".into()]
}
