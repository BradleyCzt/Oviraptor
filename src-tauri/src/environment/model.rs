//! Environment transport contracts. Compatibility exports live in `crate::models`.
use serde::{Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentDependency {
    pub name: String,
    pub command: String,
    pub version: String,
    pub available: bool,
    pub detail: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentReport {
    pub os: String,
    pub arch: String,
    pub node: String,
    pub dependencies: Vec<EnvironmentDependency>,
    pub checked_at: String,
}
