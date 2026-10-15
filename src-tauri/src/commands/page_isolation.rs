//! Project page rule preview, apply, list and disable commands.
use crate::{db, AppState};
use tauri::State;

#[tauri::command]
pub async fn preview_page_rule(
    state: State<'_, AppState>,
    input: crate::page_isolation::RuleInput,
) -> Result<crate::page_isolation::Preview, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::page_isolation::preview_on(&db::open(&path)?, input)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn apply_page_rule(
    state: State<'_, AppState>,
    input: crate::page_isolation::ApplyInput,
) -> Result<crate::page_isolation::ApplyResult, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::page_isolation::apply_on(&mut db::open(&path)?, &path, input)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn list_page_rules(
    state: State<'_, AppState>,
    project_id: i64,
) -> Result<Vec<crate::page_isolation::Rule>, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::page_isolation::list_on(&db::open(&path)?, project_id)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn disable_page_rule(
    state: State<'_, AppState>,
    project_id: i64,
    rule_id: i64,
) -> Result<crate::page_isolation::DisableResult, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::page_isolation::disable_on(&mut db::open(&path)?, &path, project_id, rule_id)
    })
    .await
    .map_err(|e| e.to_string())?
}
