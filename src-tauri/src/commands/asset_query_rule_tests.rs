use super::list_assets_on;
use crate::{asset_models::AssetQuery, db};
use rusqlite::params;
use serde_json::Value as JsonValue;
use std::{collections::HashSet, fs, path::PathBuf};
use uuid::Uuid;

struct Fixture {
    root: PathBuf,
    c: rusqlite::Connection,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("oviraptor-query-rules-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = db::initialize(&root).unwrap();
        let c = db::open(&path).unwrap();
        c.execute(
            "INSERT INTO projects(id,name) VALUES(9001,'Main'),(9002,'Control')",
            [],
        )
        .unwrap();
        for (id, tier) in [
            (1, "P1"),
            (2, "P1_强归属且近期活跃"),
            (3, "P2_强归属但入口待验证"),
            (4, "P3_名称命中待确认"),
            (5, "P10"),
            (6, ""),
            (7, "P1_wrong"),
        ] {
            c.execute("INSERT INTO assets(id,asset_key,host,title,review_tier,probe_outcome) VALUES(?1,?2,?2,?2,?3,'web_alive')",params![id,format!("example-{id}"),tier]).unwrap();
            c.execute(
                "INSERT INTO project_assets(project_id,asset_id,is_deleted) VALUES(?1,?2,?3)",
                params![if id == 7 { 9002 } else { 9001 }, id, id == 2],
            )
            .unwrap();
        }
        Self { root, c }
    }
    fn query(&self, conditions: JsonValue) -> AssetQuery {
        serde_json::from_value(serde_json::json!({"projectId":9001,"deletedView":"all","conditions":conditions,"sortBy":"host","sortDirection":"asc"})).unwrap()
    }
    fn ids(&self, q: &AssetQuery) -> Vec<i64> {
        list_assets_on(&self.c, q)
            .unwrap()
            .items
            .iter()
            .map(|a| a.id)
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn asset_query_review_tier_families_match_native_labels_without_p10_or_other_projects() {
    let f = Fixture::new();
    for (tier, ids) in [("P1", vec![1, 2]), ("P2", vec![3]), ("P3", vec![4])] {
        let mut q =
            f.query(serde_json::json!([{"field":"reviewTier","operator":"equals","value":tier}]));
        assert_eq!(f.ids(&q), ids);
        assert_eq!(
            list_assets_on(&f.c, &q).unwrap().summary.all,
            ids.len() as i64
        );
        q.conditions[0].operator = "notEquals".into();
        let others = f.ids(&q);
        assert!(!others.iter().any(|id| ids.contains(id)));
        assert_eq!(others.len() + ids.len(), 6);
    }
    let q=f.query(serde_json::json!([{"field":"reviewTier","operator":"equals","value":"P1_强归属且近期活跃"}]));
    assert_eq!(f.ids(&q), vec![2]);
    let mut q =
        f.query(serde_json::json!([{"field":"reviewTier","operator":"equals","value":"P1"}]));
    q.deleted_view = "active".into();
    assert_eq!(f.ids(&q), vec![1]);
    // Production pages have a minimum of 20 rows; cross that actual boundary.
    for id in 100..119 {
        f.c.execute("INSERT INTO assets(id,asset_key,host,review_tier) VALUES(?1,?2,?2,'P1_强归属且近期活跃')",params![id,format!("page-{id}")]).unwrap();
        f.c.execute(
            "INSERT INTO project_assets(project_id,asset_id) VALUES(9001,?1)",
            [id],
        )
        .unwrap();
    }
    q.deleted_view = "all".into();
    q.page_size = 20;
    let first = list_assets_on(&f.c, &q).unwrap();
    q.page = 2;
    let second = list_assets_on(&f.c, &q).unwrap();
    assert_eq!(
        (
            first.total,
            second.total,
            first.items.len(),
            second.items.len()
        ),
        (21, 21, 20, 1)
    );
    let ids: HashSet<_> = first
        .items
        .iter()
        .chain(&second.items)
        .map(|a| a.id)
        .collect();
    assert_eq!(ids.len(), 21);
    assert!(ids.contains(&1) && ids.contains(&2) && ids.contains(&118));
}

#[test]
fn asset_query_or_never_bypasses_search_project_or_trash_and_combines_left_to_right() {
    let f = Fixture::new();
    let mut q = f.query(
        serde_json::json!([{"field":"reviewTier","operator":"equals","value":"P1","join":"or"}]),
    );
    q.deleted_view = "active".into();
    q.search = "does-not-exist".into();
    assert!(f.ids(&q).is_empty());
    q.search = "".into();
    assert_eq!(f.ids(&q), vec![1]);
    q.conditions.push(
        serde_json::from_value(
            serde_json::json!({"field":"reviewTier","operator":"equals","value":"P2","join":"or"}),
        )
        .unwrap(),
    );
    assert_eq!(f.ids(&q), vec![1, 3]);
    q.conditions.push(serde_json::from_value(serde_json::json!({"field":"host","operator":"equals","value":"example-3","join":"and"})).unwrap());
    assert_eq!(f.ids(&q), vec![3]);
}
