//! Bounded, declarative skill package storage. Import never executes scripts,
//! installs dependencies, grants tools, or reads resources outside the selection.
mod import;
mod model;
mod store;
pub use model::{ImportInput,PackageInfo,PackagePreview};
use crate::{db,AppState};
use rusqlite::Connection;
use serde_json::Value;
use std::path::{Path,PathBuf};
use tauri::State;

pub(crate) fn initialize(db:&mut Connection)->Result<(),String>{
    db.execute_batch(include_str!("schema.sql")).map_err(|e|format!("skill_package_schema:{e}"))
}
pub(crate) fn list_resources(db:&Connection,skill_id:i64)->Result<Value,String>{store::list_resources(db,skill_id)}
pub(crate) fn read_resource(db:&Connection,skill_id:i64,path:&str,offset:usize,limit:usize)->Result<Value,String>{store::read_resource(db,skill_id,path,offset,limit)}
pub(crate) fn package_fingerprint(db:&Connection,skill_id:i64)->Result<Option<String>,String>{store::package_fingerprint(db,skill_id)}

async fn with_db<T,F>(path:PathBuf,job:F)->Result<T,String>
where T:Send+'static,F:FnOnce(&mut Connection)->Result<T,String>+Send+'static{
    tauri::async_runtime::spawn_blocking(move||{let mut db=db::open(&path)?;job(&mut db)}).await.map_err(|_|"skill_package_worker_failed".to_string())?
}

#[tauri::command]
pub async fn list_skill_packages(state:State<'_,AppState>)->Result<Vec<PackageInfo>,String>{
    with_db(state.db_path.clone(),|db|{
        // Lazy once-only registration on first Skills visit. App startup only
        // creates tables; already installed bundles are not decoded again.
        if !store::installed(db)?{store::install(db,import::bundled()?)?;}
        store::list(db)
    }).await
}
#[tauri::command]
pub async fn preview_skill_package(path:String)->Result<PackagePreview,String>{
    tauri::async_runtime::spawn_blocking(move||import::local(Path::new(&path)).map(|package|import::preview(&package))).await.map_err(|_|"skill_package_worker_failed".to_string())?
}
#[tauri::command]
pub async fn import_skill_package(state:State<'_,AppState>,input:ImportInput)->Result<PackageInfo,String>{
    with_db(state.db_path.clone(),move|db|{
        let package=import::local(Path::new(&input.path))?;
        if package.digest!=input.expected_digest{return Err("skill_package_preview_changed".into());}
        store::install(db,package)
    }).await
}
#[tauri::command]
pub async fn install_bundled_skill_package(state:State<'_,AppState>)->Result<PackageInfo,String>{
    with_db(state.db_path.clone(),|db|{
        if store::installed(db)?{return store::info(db,"every-compound-engineering");}
        store::install(db,import::bundled()?)
    }).await
}
#[tauri::command]
pub async fn set_packaged_skill_enabled(state:State<'_,AppState>,skill_id:i64,enabled:bool)->Result<(),String>{
    with_db(state.db_path.clone(),move|db|store::set_enabled(db,skill_id,enabled)).await
}
#[tauri::command]
pub async fn list_skill_package_resources(state:State<'_,AppState>,skill_id:i64)->Result<Value,String>{
    with_db(state.db_path.clone(),move|db|list_resources(db,skill_id)).await
}
#[tauri::command]
pub async fn read_skill_package_resource(state:State<'_,AppState>,skill_id:i64,path:String,offset:Option<usize>,limit:Option<usize>)->Result<Value,String>{
    with_db(state.db_path.clone(),move|db|read_resource(db,skill_id,&path,offset.unwrap_or(0),limit.unwrap_or(4096))).await
}
#[cfg(test)]
mod tests;
