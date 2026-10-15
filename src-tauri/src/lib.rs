#![recursion_limit = "256"]

mod agent_runtime;
pub mod recovery_finance;
mod artifact_import;
mod asset_logs;
mod auth_session;
mod collaboration_events;
mod commands;
mod scanning;
mod conversations;
mod skill_packages;
mod skills_catalog;
mod db;
mod execution_service;
mod installation_logs;
mod jobs;
mod llm_hook;
mod log_display;
mod log_framing;
mod models;
mod page_isolation;
#[cfg(test)]
mod local_incremental_ipc_tests;
#[path = "assets/model.rs"]
mod asset_models;
#[path = "assets/fofa.rs"]
mod asset_fofa;
#[path = "scanning/model.rs"]
mod scanning_models;
#[path = "workspaces/model.rs"]
mod workspace_models;
#[path = "configuration/model.rs"]
mod configuration_models;
#[path = "application/model.rs"]
mod application_models;
#[path = "environment/model.rs"]
mod environment_models;
#[path = "hackerone/model.rs"]
mod hackerone_models;
mod model_configuration;
mod worker_resources;
mod native_pipeline;
mod snapshot_export;
mod startup_profile;
mod tool_supply;
mod worker;

use std::{
    collections::HashMap,
    fs,
    path::Path,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize},
        Arc, Mutex,
    },
};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager,
};

pub struct AppState {
    pub db_path: PathBuf,
    pub app_data_dir: PathBuf,
    pub legacy_icon_dirs: Vec<PathBuf>,
    pub export_dir: PathBuf,
    pub cancellations: Arc<Mutex<HashMap<i64, Arc<AtomicBool>>>>,
    pub active_jobs: Arc<AtomicUsize>,
    pub worker_service: worker::WorkerServiceControl,
}

fn copy_directory(source: &Path, destination: &Path, skip_exports: bool) -> Result<(), String> {
    if !source.exists() {
        return Ok(());
    }
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        if (skip_exports && name == "exports")
            || name == "custom-app-icon.png"
            || name_text.starts_with("asset-atlas.sqlite3")
        {
            continue;
        }
        let target = destination.join(&name);
        if entry.path().is_dir() {
            copy_directory(&entry.path(), &target, false)?;
        } else if !target.exists() {
            fs::copy(entry.path(), target).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn checkpoint_and_copy_database(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_file() || destination.is_file() {
        return Ok(());
    }
    let connection = db::open(source)?;
    let checkpoint: (i64, i64, i64) = connection
        .query_row("PRAGMA wal_checkpoint(FULL)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|error| format!("旧数据库检查点失败：{error}"))?;
    if checkpoint.0 != 0 {
        return Err("旧版 Oviraptor 仍在写入数据库；请完全退出旧应用后重新启动".into());
    }
    drop(connection);
    let temporary = destination.with_extension("sqlite3.migrating");
    if temporary.exists() {
        fs::remove_file(&temporary).map_err(|error| error.to_string())?;
    }
    fs::copy(source, &temporary).map_err(|error| format!("迁移数据库失败：{error}"))?;
    fs::rename(&temporary, destination).map_err(|error| format!("启用新数据库失败：{error}"))?;
    Ok(())
}

fn prepare_oviraptor_data(home_dir: &Path) -> Result<PathBuf, String> {
    let app_data_dir = home_dir.join("oviraptor");
    fs::create_dir_all(&app_data_dir).map_err(|error| error.to_string())?;
    let database = app_data_dir.join("oviraptor.sqlite3");
    if !database.is_file() {
        let historical_dir = home_dir.join("AssetAtlas");
        let historical_database = historical_dir.join("asset-atlas.sqlite3");
        let transitional_database = app_data_dir.join("asset-atlas.sqlite3");
        if historical_database.is_file() {
            checkpoint_and_copy_database(&historical_database, &database)?;
            copy_directory(&historical_dir, &app_data_dir, true)?;
        } else if transitional_database.is_file() {
            checkpoint_and_copy_database(&transitional_database, &database)?;
        }
    }
    Ok(app_data_dir)
}

fn merge_transitional_database(app_data_dir: &Path, destination: &Path) -> Result<(), String> {
    let source = app_data_dir.join("asset-atlas.sqlite3");
    if !source.is_file() || source == destination {
        return Ok(());
    }
    let source_connection = db::open(&source)?;
    let checkpoint: (i64, i64, i64) = source_connection
        .query_row("PRAGMA wal_checkpoint(FULL)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|error| error.to_string())?;
    if checkpoint.0 != 0 {
        return Err("过渡数据库仍在使用，暂不能合并；请退出旧应用后重试".into());
    }
    drop(source_connection);

    let connection = db::open(destination)?;
    let source_text = source.to_string_lossy().to_string();
    connection
        .execute("ATTACH DATABASE ?1 AS legacy_transition", [&source_text])
        .map_err(|error| format!("读取过渡数据库失败：{error}"))?;
    let merge_result = connection.execute_batch(
        "
        INSERT OR IGNORE INTO sentinel_scans(
          id,project_id,project_name,status,current_checkpoint,task_path,previous_scan_id,
          llm_requests,input_tokens,output_tokens,cached_tokens,total_tokens,scan_type,
          task_name,source_path,skill_names,created_at,updated_at
        )
        SELECT id,project_id,project_name,status,current_checkpoint,task_path,previous_scan_id,
          llm_requests,input_tokens,output_tokens,cached_tokens,total_tokens,scan_type,
          task_name,source_path,skill_names,created_at,updated_at
        FROM legacy_transition.sentinel_scans;
        INSERT OR REPLACE INTO sentinel_checkpoints(scan_id,url,stage,raw_json,updated_at)
        SELECT scan_id,url,stage,raw_json,updated_at
        FROM legacy_transition.sentinel_checkpoints;
        INSERT OR IGNORE INTO sentinel_findings(
          scan_id,target_url,stage,kind,record_key,title,severity,record_json,updated_at
        )
        SELECT scan_id,target_url,stage,kind,record_key,title,severity,record_json,updated_at
        FROM legacy_transition.sentinel_findings;
        ",
    );
    let _ = connection.execute_batch("DETACH DATABASE legacy_transition");
    merge_result.map_err(|error| format!("合并过渡扫描数据失败：{error}"))?;
    drop(connection);

    let backup_dir = app_data_dir.join("database-backups");
    fs::create_dir_all(&backup_dir).map_err(|error| error.to_string())?;
    let backup = backup_dir.join("pre-oviraptor.sqlite3.bak");
    if !backup.exists() {
        fs::rename(&source, &backup).map_err(|error| format!("归档旧数据库失败：{error}"))?;
        for suffix in ["-wal", "-shm"] {
            let auxiliary = app_data_dir.join(format!("asset-atlas.sqlite3{suffix}"));
            if auxiliary.exists() {
                let _ = fs::remove_file(auxiliary);
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn set_macos_dock_icon(app: &tauri::AppHandle, bytes: Option<Vec<u8>>) -> Result<(), String> {
    app.run_on_main_thread(move || {
        use objc2::{AllocAnyThread, MainThreadMarker};
        use objc2_app_kit::{NSApplication, NSImage};
        use objc2_foundation::NSData;

        let marker = unsafe { MainThreadMarker::new_unchecked() };
        let application = NSApplication::sharedApplication(marker);
        if let Some(bytes) = bytes {
            let data = NSData::with_bytes(&bytes);
            if let Some(icon) = NSImage::initWithData(NSImage::alloc(), &data) {
                unsafe { application.setApplicationIconImage(Some(&icon)) };
            }
        } else {
            unsafe { application.setApplicationIconImage(None) };
        }
    })
    .map_err(|error| error.to_string())
}

#[cfg(not(target_os = "macos"))]
pub fn set_macos_dock_icon(_: &tauri::AppHandle, _: Option<Vec<u8>>) -> Result<(), String> {
    Ok(())
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Some(exit_code) = tool_supply::container_parser::guardian_entry() {
        std::process::exit(exit_code);
    }
    let context = tauri::generate_context!();
    if let Err(error) = startup_profile::preflight(&context.config().identifier) {
        eprintln!("Oviraptor acceptance profile rejected: {error}");
        std::process::exit(2);
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let acceptance = startup_profile::acceptance_root()?;
            let (app_data_dir, export_dir, legacy_icon_dirs) = if let Some(root) = &acceptance {
                let (data, exports) = startup_profile::isolated_directories(root)?;
                (data, exports, Vec::new())
            } else {
                let legacy_dir = app
                    .path()
                    .app_data_dir()
                    .map_err(|error| error.to_string())?;
                let previous_app_dir = app
                    .path()
                    .data_dir()
                    .map_err(|error| error.to_string())?
                    .join("com.assetatlas.desktop");
                let home_dir = app.path().home_dir().map_err(|error| error.to_string())?;
                let data = prepare_oviraptor_data(&home_dir)?;
                let exports = app
                    .path()
                    .download_dir()
                    .map_err(|error| error.to_string())?
                    .join("oviraptor");
                fs::create_dir_all(&exports).map_err(|error| error.to_string())?;
                copy_directory(&legacy_dir, &data, true)?;
                copy_directory(&legacy_dir.join("exports"), &exports, false)?;
                (data, exports, vec![legacy_dir, previous_app_dir])
            };
            crate::scanning::log_events::install_nest_runner_log_bus(app.handle().clone());
            let db_path = db::initialize(&app_data_dir)?;
            if acceptance.is_none() {
                merge_transitional_database(&app_data_dir, &db_path)?;
            }
            let icon = app.default_window_icon().cloned();
            let tray_icon =
                tauri::image::Image::from_bytes(include_bytes!("../icons/tray-icon.png"))
                    .map_err(|error| error.to_string())?;
            if let (Some(window), Some(icon)) = (app.get_webview_window("main"), icon.clone()) {
                let _ = window.set_icon(icon);
            }
            let open_item = MenuItem::with_id(app, "tray-open", "打开界面", true, None::<&str>)?;
            let assets_item =
                MenuItem::with_id(app, "tray-assets", "查看资产", true, None::<&str>)?;
            let agent_tasks_item =
                MenuItem::with_id(app, "tray-agent-tasks", "查看任务中心", true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "tray-quit", "退出应用", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &open_item,
                    &assets_item,
                    &agent_tasks_item,
                    &separator,
                    &quit_item,
                ],
            )?;
            let tray_builder = TrayIconBuilder::with_id("main")
                .tooltip("Oviraptor · 后台运行")
                .menu(&menu)
                .icon(tray_icon)
                .icon_as_template(true)
                .show_menu_on_left_click(true);
            tray_builder
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "tray-open" => show_main_window(app),
                    "tray-assets" => {
                        show_main_window(app);
                        let _ = app.emit("tray-navigate", "assets");
                    }
                    "tray-agent-tasks" => {
                        show_main_window(app);
                        let _ = app.emit("tray-navigate", "agent-tasks");
                    }
                    "tray-quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            app.manage(collaboration_events::start_event_pump(
                app.handle().clone(),
                db_path.clone(),
            )?);
            app.manage(AppState {
                db_path,
                app_data_dir,
                legacy_icon_dirs,
                export_dir,
                cancellations: Arc::new(Mutex::new(HashMap::new())),
                active_jobs: Arc::new(AtomicUsize::new(0)),
                worker_service: worker::WorkerServiceControl::default(),
            });
            let _ = worker::restart_worker_service(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            } else if window.label().starts_with("oviraptor-auth-")
                && matches!(event, tauri::WindowEvent::CloseRequested { .. })
            {
                auth_session::browser_auth_window_closed(window.app_handle(), window.label());
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::application_overview::dashboard_stats,
            commands::workspace_projects::list_projects,
            conversations::get_conversation_access,
            conversations::set_conversation_access,
            conversations::list_conversation_access_skills,
            conversations::grant_conversation_directory,
            conversations::revoke_conversation_directory,
            conversations::list_conversations,
            conversations::create_conversation,
            conversations::read_conversation,
            conversations::update_conversation,
            conversations::set_conversation_links,
            conversations::send_conversation_message,
            conversations::cancel_conversation_reply,
            conversations::delete_conversation,
            skill_packages::list_skill_packages,
            skill_packages::preview_skill_package,
            skill_packages::import_skill_package,
            skill_packages::install_bundled_skill_package,
            skill_packages::set_packaged_skill_enabled,
            skill_packages::list_skill_package_resources,
            skill_packages::read_skill_package_resource,
            commands::workspace_projects::save_project,
            commands::workspace_projects::project_impact,
            commands::workspace_projects::archive_project,
            commands::workspace_projects::delete_project,
            auth_session::open_browser_auth_session,
            auth_session::finish_browser_auth_session,
            auth_session::list_browser_auth_sessions,
            auth_session::list_sentinel_scan_auth_sessions,
            auth_session::get_sentinel_scan_identity_material,
            auth_session::validate_browser_auth_session,
            auth_session::delete_browser_auth_session,
            commands::application_settings::get_app_settings,
            commands::application_settings::get_app_icon_data_url,
            commands::application_settings::save_app_settings,
            commands::application_settings::save_app_icon,
            commands::application_settings::reset_app_icon,
            commands::assets::startup_status,
            commands::assets::acknowledge_interrupted_run,
            commands::workspace_config_profiles::list_config_profiles,
            commands::workspace_config_profiles::save_config_profile,
            commands::workspace_config_profiles::delete_config_profile,
            worker::get_local_worker_settings,
            worker::save_local_worker_settings,
            worker::list_worker_nodes,
            worker::save_worker_node,
            worker::delete_worker_node,
            worker::test_worker_node,
            worker::list_remote_worker_scans,
            worker::get_remote_worker_environment,
            worker::control_remote_worker_scan,
            worker::sync_worker_node,
            commands::assets::import_targets,
            commands::assets::list_targets,
            commands::assets::remove_target,
            commands::assets::list_assets,
            commands::assets::mark_asset_entry_opened,
            commands::assets::add_content_rule,
            commands::page_isolation::preview_page_rule,
            commands::page_isolation::apply_page_rule,
            commands::page_isolation::list_page_rules,
            commands::asset_rule_catalog::get_asset_rule_catalog,
            commands::page_isolation::disable_page_rule,
            commands::assets::update_decision,
            commands::assets::update_asset_decisions,
            commands::ownership::get_asset_ownership_profile,
            commands::ownership::save_asset_ownership_profile,
            commands::ownership::get_asset_ownership_summary,
            commands::ownership::assess_asset_ownership,
            commands::ownership::update_asset_ownership,
            commands::ownership::get_project_ownership_bootstrap,
            commands::ownership::preview_project_ownership_suggestions,
            commands::ownership::save_project_ownership_directory,
            commands::ownership::start_ownership_analysis,
            commands::ownership::read_ownership_analysis,
            commands::ownership::cancel_ownership_analysis,
            commands::ownership::page_ownership_groups,
            commands::ownership::page_ownership_group_members,
            commands::ownership::preview_ownership_group_decision,
            commands::ownership::apply_ownership_group_decision,
            commands::ownership::read_ownership_operation,
            commands::ownership::preview_ownership_group_revert,
            commands::ownership::apply_ownership_group_revert,
            commands::ownership::revoke_ownership_group_authorization,
            commands::ownership::confirm_asset_ownership,
            commands::ownership::authorize_asset_ownership,
            commands::exposure::exposure_summary,
            commands::exposure::list_exposure_findings,
            commands::exposure::page_exposure_findings,
            commands::exposure::page_exposure_runs,
            commands::exposure::list_exposure_source_results_for_project,
            commands::exposure::read_exposure_finding,
            commands::exposure::exposure_scope_summary,
            commands::exposure::exposure_known_limits,
            commands::exposure::resume_exposure_scan,
            commands::exposure::page_exposure_pending_discoveries,
            commands::exposure::read_exposure_evidence,
            commands::exposure::get_exposure_settings,
            commands::exposure::save_exposure_settings,
            commands::exposure::list_exposure_setting_versions,
            commands::exposure::page_exposure_clusters,
            commands::exposure::page_exposure_cluster_members,
            commands::exposure::verify_exposure_export,
            commands::exposure::review_exposure_finding_scoped,
            commands::exposure::list_exposure_review_trail,
            commands::exposure::save_exposure_remediation,
            commands::exposure::page_exposure_remediations,
            commands::exposure::start_exposure_retest,
            commands::exposure::read_exposure_retest_outcome,
            commands::exposure::mark_exposure_remediation,
            commands::exposure::exposure_review_taxonomy,
            commands::exposure::save_exposure_finding,
            commands::exposure::start_exposure_scan,
            commands::exposure::list_exposure_runs,
            commands::exposure::list_exposure_source_results,
            commands::exposure::cancel_exposure_scan,
            commands::exposure::export_exposure_findings,
            commands::exposure::page_exposure_incidents,
            commands::exposure::read_exposure_incident,
            commands::exposure::page_exposure_incident_locations,
            commands::exposure::exposure_coverage_drilldown,
            commands::exposure::act_on_exposure_incident,
            commands::exposure::propose_exposure_incidents,
            commands::exposure::import_exposure_documents,
            commands::exposure::grade_exposure_intel,
            commands::exposure::reclassify_exposure_finding,
            commands::assets::soft_delete_assets,
            commands::assets::soft_delete_asset_selections,
            commands::assets::list_runs,
            commands::assets::list_logs,
            commands::assets::list_asset_events,
            commands::hackerone::list_hackerone_programs,
            commands::hackerone::get_hackerone_detail,
            commands::hackerone::set_hackerone_bookmark,
            commands::hackerone::list_hackerone_events,
            commands::hackerone::sync_hackerone,
            commands::hackerone::add_hackerone_scopes_to_project,
            commands::environment::check_environment,
            commands::tool_administrator::approve_tool_inventory,
            commands::tool_administrator::revoke_tool_inventory,
            commands::tool_administrator::read_tool_approval_inventory,
            commands::environment::install_environment_dependencies,
            commands::environment::list_environment_install_logs,
            commands::environment::get_environment_preparation_status,
            commands::environment::recover_environment_preparation,
            scanning::create_sentinel_scan,
            scanning::create_sentinel_url_scan,
            scanning::create_sentinel_homogeneous_url_scan,
            scanning::create_sentinel_url_scan_with_tool_selection,
            commands::model_profiles::test_model_profile,
            asset_fofa::test_fofa_api,
            skills_catalog::instructions::list_agent_instructions,
            scanning::list_capability_bundles,
            scanning::list_role_config_drafts,
            scanning::save_role_config_draft,
            scanning::role_config_draft_status_label,
            scanning::save_authorization_control,
            scanning::get_authorization_control_setup,
            skills_catalog::instructions::save_agent_instruction,
            skills_catalog::instructions::delete_agent_instruction,
            skills_catalog::instructions::export_agent_instructions,
            skills_catalog::instructions::import_agent_instructions,
            skills_catalog::instructions::import_sec_skill_knowledge,
            skills_catalog::sources::ingest_agent_knowledge_source,
            scanning::list_agent_traces,
            scanning::get_agent_trace,
            skills_catalog::knowledge::list_agent_knowledge,
            skills_catalog::knowledge::list_agent_learning_candidates,
            scanning::generate_agent_learning_candidate,
            skills_catalog::knowledge::review_agent_learning_candidate,
            skills_catalog::knowledge::delete_agent_learning_candidate,
            skills_catalog::knowledge::apply_agent_learning_candidate,
            scanning::analyze_agent_trace,
            scanning::aggregate_agent_knowledge,
            skills_catalog::knowledge::delete_agent_knowledge,
            skills_catalog::knowledge::convert_agent_knowledge_to_instruction,
            skills_catalog::knowledge::refine_agent_instruction_with_knowledge,
            skills_catalog::knowledge::export_agent_knowledge,
            skills_catalog::knowledge::import_agent_knowledge,
            scanning::list_security_rule_packs,
            scanning::save_security_rule_pack,
            scanning::delete_security_rule_pack,
            scanning::sync_security_rule_pack,
            scanning::start_workbench_scan,
            scanning::rescan_workbench_scan,
            scanning::rescan_sentinel_scan,
            scanning::confirm_sentinel_scan,
            scanning::pause_sentinel_scan,
            scanning::recover_native_source_pause_result,
            scanning::resume_sentinel_scan,
            scanning::cancel_sentinel_scan,
            scanning::delete_sentinel_scan,
            scanning::list_sentinel_scans,
            scanning::search_sentinel_scan_page,
            scanning::archive_sentinel_scan,
            scanning::list_sentinel_scan_attempts,
            scanning::list_sentinel_vulnerability_scan_ids,
            scanning::get_sentinel_runner_log,
            scanning::read_sentinel_runner_log,
            scanning::read_native_process_log,
            scanning::read_native_sdk_log,
            scanning::read_native_mapper_prepared_history,
            scanning::search_sentinel_scan_ids,
            scanning::list_sentinel_targets,
            scanning::list_sentinel_fuse_zone,
            scanning::save_sentinel_fuse_review,
            scanning::remove_sentinel_fuse_entry,
            scanning::list_sentinel_checkpoints,
            scanning::list_sentinel_findings,
            scanning::list_historical_import_previews,
            scanning::list_historical_import_runs,
            scanning::list_historical_bundle_previews,
            scanning::get_agent_target_execution,
            scanning::get_agent_request_reviews,
            scanning::record_agent_request_review,
            scanning::get_native_scan_status,
            scanning::get_native_budget_diagnostics,
            scanning::get_native_root_cost_reconciliations,
            scanning::confirm_native_root_cost_reconciliation,
            scanning::preview_native_root_financial_continuation,
            scanning::preview_native_single_continuation,
            scanning::preview_native_protection_recovery,
            scanning::discover_native_protection_recovery_sources,
            scanning::preview_native_protection_recovery_proposal,
            scanning::approve_native_protection_recovery,
            scanning::preview_native_priced_protection_recovery,
            scanning::approve_native_priced_protection_recovery,
            scanning::get_native_priced_protection_recovery_approval,
            scanning::find_native_priced_protection_recovery_approval,
            scanning::cancel_native_priced_protection_recovery_approval,
            scanning::get_native_priced_protection_recovery_cancellation,
            scanning::get_native_protection_recovery_approval,
            scanning::create_native_protection_recovery_draft,
            scanning::get_native_protection_recovery_draft,
            scanning::get_native_protection_recovery_cancellation,
            scanning::get_native_protection_recovery_cancellation_by_request,
            scanning::approve_native_root_financial_continuation,
            scanning::get_native_root_financial_continuation,
            scanning::discover_native_root_financial_continuation,
            scanning::get_native_scan_timeline_page,
            scanning::get_agent_dialog_view,
            scanning::save_agent_dialog_view,
            scanning::get_agent_dialog_selection,
            scanning::save_agent_dialog_selection,
            scanning::get_agent_dialog_task,
            scanning::recover_never_dispatched_web_attempt,
            scanning::close_never_dispatched_web_attempt,
            scanning::preview_web_administrative_closure,
            scanning::close_web_task_administratively,
            scanning::preview_web_closure_handoff,
            scanning::create_web_closure_handoff,
            scanning::preview_agent_gap_followup,
            scanning::create_agent_gap_followup,
            scanning::get_agent_gap_followup_submission,
            scanning::release_agent_gap_followup_submission,
            scanning::get_native_attempt_mailbox_history,
            scanning::get_native_attempt_tool_history,
            scanning::get_native_attempt_execution_history,
            scanning::get_native_source_findings,
            scanning::export_native_source_findings,
            scanning::draft_scan_directive,
            scanning::post_scan_directive,
            scanning::confirm_scan_directive,
            scanning::revise_scan_directive,
            scanning::reject_scan_directive,
            scanning::reconcile_scan_directive_receipt,
            scanning::reconcile_historical_scan_directive_receipt,
            scanning::cancel_scan_directive,
            scanning::list_sentinel_opportunities,
            scanning::update_sentinel_opportunity_status,
            scanning::get_investigation_graph,
            scanning::list_investigation_hypotheses,
            scanning::update_investigation_hypothesis,
            scanning::set_investigation_mutation_approval,
            scanning::replay_investigation_request,
            scanning::save_investigation_validation,
            scanning::list_investigation_validations,
            scanning::investigation_overview,
            scanning::list_appsec_scan_result,
            scanning::sentinel_overview_stats,
            scanning::list_sentinel_validations,
            scanning::list_all_sentinel_validations,
            scanning::list_sentinel_validation_work_items,
            scanning::save_sentinel_validation,
            scanning::export_sentinel_results,
            scanning::import_sentinel_results,
            scanning::export_sentinel_project,
            scanning::import_sentinel_project,
            scanning::get_artifact_import_status,
            commands::asset_export::export_assets,
            jobs::start_job,
            jobs::resume_job,
            jobs::cancel_job,
        ])
        .build(context)
        .expect("error while building Oviraptor")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(pump) = app.try_state::<collaboration_events::EventPumpControl>() {
                    pump.stop();
                }
            }
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                show_main_window(app);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::prepare_oviraptor_data;
    use std::fs;

    fn test_home() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("oviraptor-data-dir-test-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn migrates_a_historical_database_into_oviraptor() {
        let home = test_home();
        let historical_dir = home.join("AssetAtlas");
        fs::create_dir_all(&historical_dir).unwrap();
        let historical_database = historical_dir.join("asset-atlas.sqlite3");
        drop(rusqlite::Connection::open(&historical_database).unwrap());

        let result = prepare_oviraptor_data(&home).unwrap();
        assert_eq!(result, home.join("oviraptor"));
        assert!(result.join("oviraptor.sqlite3").is_file());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn uses_oviraptor_directory_for_new_users() {
        let home = test_home();

        assert_eq!(
            prepare_oviraptor_data(&home).unwrap(),
            home.join("oviraptor")
        );
    }
}
