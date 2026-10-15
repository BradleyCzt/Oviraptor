//! Read-only inventory of project page rules and legacy content rules.
use crate::{db, AppState};
use tauri::State;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedContentRule {
    id: i64,
    keyword: String,
    enabled: bool,
    created_at: String,
    example_title: String,
    example_url: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetRuleCatalog {
    project_rules: Vec<crate::page_isolation::Rule>,
    legacy_rules: Vec<SavedContentRule>,
}

fn asset_rule_catalog_on(connection: &rusqlite::Connection) -> Result<AssetRuleCatalog, String> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    let projects = {
        let mut s = transaction
            .prepare("SELECT DISTINCT project_id FROM asset_page_rules ORDER BY project_id")
            .map_err(|e| e.to_string())?;
        let rows = s
            .query_map([], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    let mut project_rules = Vec::new();
    for project in projects {
        project_rules.extend(crate::page_isolation::list_on(&transaction, project)?);
    }
    let legacy_rules = {
        let mut s=transaction.prepare("SELECT r.id,r.keyword,r.enabled,r.created_at,COALESCE(a.title,''),COALESCE(NULLIF(a.link,''),a.host,'') FROM content_rules r LEFT JOIN assets a ON a.id=r.source_asset_id ORDER BY r.id DESC").map_err(|e|e.to_string())?;
        let rows = s
            .query_map([], |r| {
                Ok(SavedContentRule {
                    id: r.get(0)?,
                    keyword: r.get(1)?,
                    enabled: r.get(2)?,
                    created_at: r.get(3)?,
                    example_title: r.get(4)?,
                    example_url: r.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(AssetRuleCatalog {
        project_rules,
        legacy_rules,
    })
}

#[tauri::command]
pub async fn get_asset_rule_catalog(
    state: State<'_, AppState>,
) -> Result<AssetRuleCatalog, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || asset_rule_catalog_on(&db::open(&path)?))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::asset_rule_catalog_on;
    use crate::db;
    use std::{fs, path::PathBuf};

    struct Fixture {
        root: PathBuf,
        c: rusqlite::Connection,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("oviraptor-rule-catalog-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            let path = db::initialize(&root).unwrap();
            let c = db::open(&path).unwrap();
            c.execute("INSERT INTO projects(id,name) VALUES(9001,'Main'),(9002,'Control')", []).unwrap();
            c.execute("INSERT INTO assets(id,asset_key,host,title) VALUES(1,'example-1','example-1','example-1'),(7,'example-7','example-7','control')", []).unwrap();
            c.execute("INSERT INTO project_assets(project_id,asset_id) VALUES(9001,1),(9002,7)", []).unwrap();
            Self { root, c }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn asset_rule_catalog_exposes_legacy_and_all_project_rules_read_only_even_disabled() {
        let f = Fixture::new();
        f.c.execute("INSERT INTO content_rules(keyword,normalized_keyword,source_asset_id,enabled) VALUES('Example','example',1,1),('Old disabled','old disabled',NULL,0)",[]).unwrap();
        f.c.execute("INSERT INTO asset_page_rules(project_id,source_asset_id,kind,value,enabled) VALUES(9001,1,'title_contains','example',1),(9002,7,'title_contains','control',0)",[]).unwrap();
        let before = f.c.total_changes();
        f.c.execute_batch("PRAGMA query_only=ON").unwrap();
        let catalog = asset_rule_catalog_on(&f.c).unwrap();
        assert_eq!(catalog.project_rules.len(), 2);
        assert!(catalog
            .project_rules
            .iter()
            .any(|r| r.project_id == 9002 && !r.enabled));
        assert_eq!(catalog.legacy_rules.len(), 2);
        assert!(catalog
            .legacy_rules
            .iter()
            .any(|r| r.example_title == "example-1" && r.enabled));
        assert!(catalog
            .legacy_rules
            .iter()
            .any(|r| r.keyword == "Old disabled" && !r.enabled && r.example_title.is_empty()));
        assert_eq!(f.c.total_changes(), before);
    }
}
