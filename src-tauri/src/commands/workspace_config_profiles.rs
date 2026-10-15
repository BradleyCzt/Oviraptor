//! Configuration persistence. Preserve stored settings using the database policy;
//! profile storage does not resolve credentials or start business execution.
use crate::{db, models::{ConfigProfile, ConfigProfileInput}, AppState};
use rusqlite::{params, OptionalExtension};
use serde_json::Value as JsonValue;
use tauri::State;
#[tauri::command]
pub fn list_config_profiles(state: State<AppState>) -> Result<Vec<ConfigProfile>, String> {
    let connection = db::open(&state.db_path)?;
    read_config_profiles(&connection)
}

pub(super) fn read_config_profiles(connection: &rusqlite::Connection) -> Result<Vec<ConfigProfile>, String> {
    let mut statement = connection.prepare(
        "SELECT id,name,description,is_default,settings_json,created_at,updated_at FROM config_profiles ORDER BY is_default DESC, updated_at DESC"
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(ConfigProfile {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                is_default: row.get::<_, i64>(3)? != 0,
                settings: db::normalize_settings(&serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or_else(|_| serde_json::json!({}))),
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn save_config_profile(
    state: State<'_, AppState>,
    input: ConfigProfileInput,
) -> Result<i64, String> {
    let mut connection = db::open(&state.db_path)?;
    persist_config_profile(&mut connection, input)
}

pub(super) fn persist_config_profile(
    connection: &mut rusqlite::Connection,
    input: ConfigProfileInput,
) -> Result<i64, String> {
    if input.name.trim().is_empty() {
        return Err("配置名称不能为空".into());
    }
    if !input.settings.is_object() {
        return Err("配置 settings 必须是 JSON 对象".into());
    }
    let settings = db::normalize_settings(&input.settings);
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    if input.is_default {
        transaction
            .execute("UPDATE config_profiles SET is_default=0", [])
            .map_err(|error| error.to_string())?;
    }
    let id = if let Some(id) = input.id {
        let raw:String=transaction.query_row("SELECT settings_json FROM config_profiles WHERE id=?1",[id],|row|row.get(0))
            .optional().map_err(|error|error.to_string())?.ok_or("配置不存在，未保存任何更改")?;
        let stored:JsonValue=serde_json::from_str(&raw).map_err(|_|"已保存的配置无法解析，不能隐式替换")?;
        let settings=db::settings_for_update(&stored,&input.settings)?;
        let changed = transaction.execute(
            "UPDATE config_profiles SET name=?1,description=?2,is_default=?3,settings_json=?4,updated_at=datetime('now','localtime') WHERE id=?5",
            params![input.name.trim(), input.description.trim(), input.is_default as i64, settings.to_string(), id],
        ).map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("配置不存在，未保存任何更改".into());
        }
        id
    } else {
        transaction.execute(
            "INSERT INTO config_profiles(name,description,is_default,settings_json) VALUES(?1,?2,?3,?4)",
            params![input.name.trim(), input.description.trim(), input.is_default as i64, settings.to_string()],
        ).map_err(|error| error.to_string())?;
        transaction.last_insert_rowid()
    };
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(id)
}

#[tauri::command]
pub fn delete_config_profile(state: State<AppState>, profile_id: i64) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    delete_config_profile_inner(&connection,profile_id)
}

pub(super) fn delete_config_profile_inner(connection: &rusqlite::Connection,profile_id:i64)->Result<(),String> {
    let tx=rusqlite::Transaction::new_unchecked(connection,rusqlite::TransactionBehavior::Immediate)
        .map_err(|error|error.to_string())?;
    let (is_default,raw):(i64,String)=tx.query_row("SELECT is_default,settings_json FROM config_profiles WHERE id=?1",[profile_id],
        |row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(|error|error.to_string())?.ok_or("配置方案不存在")?;
    if is_default!=0 {return Err("系统默认配置不能删除".into());}
    let stored:JsonValue=serde_json::from_str(&raw).map_err(|_|"已保存的配置无法解析，不能隐式删除")?;
    let retained=db::settings_for_update(&stored,&serde_json::json!({}))?;
    if retained.as_object().is_none_or(|fields|!fields.is_empty()) {
        return Err("retired_settings_require_confirmed_cleanup".into());
    }
    let changed=tx.execute("DELETE FROM config_profiles WHERE id=?1",[profile_id]).map_err(|error|error.to_string())?;
    let remains:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM config_profiles WHERE id=?1)",[profile_id],|row|row.get(0)).map_err(|error|error.to_string())?;
    if changed!=1 || remains {return Err("配置删除未确认，已回滚".into());}
    tx.commit().map_err(|error|error.to_string())
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::configuration_models::{ConfigProfile, ConfigProfileInput};

    pub(crate) fn read_config_profiles(
        connection: &rusqlite::Connection,
    ) -> Result<Vec<ConfigProfile>, String> {
        super::read_config_profiles(connection)
    }

    pub(crate) fn persist_config_profile(
        connection: &mut rusqlite::Connection,
        input: ConfigProfileInput,
    ) -> Result<i64, String> {
        super::persist_config_profile(connection, input)
    }

    pub(crate) fn delete_config_profile_inner(
        connection: &rusqlite::Connection,
        profile_id: i64,
    ) -> Result<(), String> {
        super::delete_config_profile_inner(connection, profile_id)
    }
}
