//! Synthetic model callbacks drive the real scoped tools and temporary SQLite
//! database. These tests never call a provider or a target.
use super::*;
use crate::conversations::store;

fn inventory(f: &Fixture) {
    let db = db::open(&f.path).unwrap();
    for n in 0..26_i64 {
        let id = 1000 + n * 137; // IDs are deliberately sparse, not a count.
        let status = match n {
            0..=8 => "200",
            9..=14 => "302",
            15..=17 => "403",
            18..=19 => "",
            _ => "future_status",
        };
        let outcome = match n {
            0..=8 => "web_alive",
            9..=14 => "web_restricted",
            15..=17 => "tcp_alive_non_http",
            18..=19 => "",
            _ => "future_probe",
        };
        let decision = match n {
            14..=16 => "confirmed",
            17..=18 => "rejected",
            19 => "not_applicable",
            20 => "uncertain",
            21 => "",
            22 => "future_decision",
            _ => "pending",
        };
        db.execute("INSERT INTO assets(id,asset_key,company,host,link,ip,domain,title,protocol,port,status_code,probe_outcome,extra_json) VALUES(?1,?2,'Fixture Group',?3,?4,'198.51.100.30','scope.example',?5,'https','443',?6,?7,'{\"secret\":\"synthetic-private-extra\"}')",params![id,format!("scoped-key-{n}"),format!("host-{n}.example"),format!("https://host-{n}.example/"),format!("Scoped asset {n}"),status,outcome]).unwrap();
        let deleted = if n < 23 {
            0
        } else if n < 25 {
            1
        } else {
            7
        };
        db.execute("INSERT INTO project_assets(project_id,asset_id,decision,is_deleted) VALUES(901,?1,?2,?3)",params![id,decision,deleted]).unwrap();
    }
    db.execute("INSERT INTO assets(id,asset_key,host,title) VALUES(900001,'foreign','foreign.example','Other project private')",[]).unwrap();
    db.execute(
        "INSERT INTO project_assets(project_id,asset_id) VALUES(902,900001)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO sentinel_targets(project_id,asset_id,url) VALUES(901,1000,'https://host-0.example/'),(901,1000,'https://host-0.example/second')",[]).unwrap();
}
fn data(request: &ModelRequest) -> Value {
    let tool = request
        .messages
        .iter()
        .rev()
        .find(|message| message["role"] == "tool")
        .unwrap();
    serde_json::from_str::<Value>(tool["content"].as_str().unwrap()).unwrap()
}
fn count_input(f: &Fixture, item: &Conversation) -> SendInput {
    let mut value = input(f, item);
    value.content = "这个项目当前有多少资产？请说明未知状态。".into();
    value
}
fn total_buckets(value: &Value) -> i64 {
    value
        .as_object()
        .unwrap()
        .values()
        .map(|n| n.as_i64().unwrap())
        .sum()
}

#[test]
fn chinese_quantity_question_gets_exact_scoped_statistics_and_unknown_buckets_in_real_dispatch() {
    let f = Fixture::new();
    let item = f.create();
    grant_assets(&f, &item);
    inventory(&f);
    let mut rounds = 0;
    let receipt=send_with(&f.path,count_input(&f,&item),|_|Ok(profile()),|_,request,_|{
        rounds+=1;
        if rounds==1 {
            assert!(request.tools.iter().any(|tool|tool.name=="assets_get_stats"));
            let instructions=request.messages[0]["content"].as_str().unwrap();
            for requirement in ["assets_get_stats","Never infer totals","unknown/unclassified","Dimensions","activeTotal"] {
                assert!(instructions.to_lowercase().contains(&requirement.to_lowercase()),"missing counting instruction {requirement}");
            }
            return Ok(call("assets_get_stats",json!({})));
        }
        let result=data(request); assert_eq!(result["ok"],true);
        let stats=&result["data"];
        assert_eq!(stats["projectTotal"],26); assert_eq!(stats["activeTotal"],23);
        assert_eq!(stats["trashedTotal"],2); assert_eq!(stats["unknownArchiveState"],1);
        assert_eq!(stats["filteredTotal"],23); assert_eq!(stats["scope"]["projectId"],901);
        assert_eq!(stats["scope"]["filters"]["deletedView"],"active");
        assert_eq!(stats["scope"]["searchSemantics"],"assets_list_like_8_fields");
        for dimension in ["byDecision","byProbeOutcome","byHttpStatusClass"] {assert_eq!(total_buckets(&stats[dimension]),23);}
        assert_eq!(stats["byDecision"]["unknown"],1);
        assert_eq!(stats["byProbeOutcome"]["unclassified"],2); assert_eq!(stats["byProbeOutcome"]["unknown"],3);
        assert_eq!(stats["byHttpStatusClass"]["unclassified"],2); assert_eq!(stats["byHttpStatusClass"]["unknown"],3);
        assert!(chrono::DateTime::parse_from_rfc3339(stats["asOf"].as_str().unwrap()).is_ok());
        assert!(stats["readConsistency"].as_str().is_some_and(|value|!value.is_empty()));
        assert!(stats["breakdownSemantics"].as_str().unwrap().contains("overlap"));
        assert!(!result.to_string().contains("synthetic-private-extra"));
        assert!(!result.to_string().contains("Other project private"));
        Ok(ModelResponse{text:"当前项目有 23 条未回收资产；包含回收共 26 条。探测结果有 2 条未分类、3 条未知。各统计维度可能重叠，不能相加；这是已保存库存，不表示当前在线。".into(),..Default::default()})
    }).unwrap();
    assert_eq!(rounds, 2);
    assert_eq!(receipt.reply.unwrap().status, "completed");
    let saved = f.read(&item);
    assert!(saved.messages[1].content.contains("23 条"));
    assert_eq!(saved.tool_activity.len(), 1);
    assert_eq!(saved.tool_activity[0].name, "assets_get_stats");
    assert!(saved.tool_activity[0].summary.contains("23 active"));
}

#[test]
fn quantity_question_can_use_search_exact_total_instead_of_guessing_from_twenty_sparse_ids() {
    let f = Fixture::new();
    let item = f.create();
    grant_assets(&f, &item);
    inventory(&f);
    let mut rounds = 0;
    let receipt=send_with(&f.path,count_input(&f,&item),|_|Ok(profile()),|_,request,_|{
        rounds+=1;
        if rounds==1 {return Ok(call("assets_search",json!({"limit":20})));}
        let result=data(request); assert_eq!(result["ok"],true);
        let page=&result["data"];
        assert_eq!(page["total"],23); assert_eq!(page["returned"],20);
        assert_eq!(page["items"].as_array().unwrap().len(),20); assert_eq!(page["hasMore"],true);
        assert_eq!(page["itemsArePage"],true); assert!(page["nextCursor"].as_str().is_some_and(|v|!v.is_empty()));
        assert_eq!(page["scope"]["projectId"],901); assert_eq!(page["order"],"asset_id_asc");
        let ids:Vec<_>=page["items"].as_array().unwrap().iter().map(|row|row["id"].as_i64().unwrap()).collect();
        assert_eq!(ids[0],1000); assert!(ids[19]-ids[0]>2000);
        assert!(!result.to_string().contains("foreign.example"));
        Ok(ModelResponse{text:"当前项目精确匹配 23 条未回收资产，本次仅列出 20/23 条。ID 不连续，不用 ID 跨度估算数量。".into(),..Default::default()})
    }).unwrap();
    assert_eq!(rounds, 2);
    assert_eq!(receipt.reply.unwrap().status, "completed");
    let saved = f.read(&item);
    assert!(saved.messages[1].content.contains("20/23"));
    assert!(saved.tool_activity[0].summary.contains("20 of 23"));
}

#[test]
fn real_dispatch_search_cursor_keeps_scope_and_filter_totals_and_never_discloses_recycled_rows() {
    let f = Fixture::new();
    let item = f.create();
    grant_assets(&f, &item);
    inventory(&f);
    let db = db::open(&f.path).unwrap();
    let first = tools::execute(
        &db,
        &item,
        "not-running",
        "assets_search",
        &json!({"limit":20}),
    )
    .unwrap();
    let cursor = first["nextCursor"].as_str().unwrap();
    let second = tools::execute(
        &db,
        &item,
        "not-running",
        "assets_search",
        &json!({"limit":20,"cursor":cursor}),
    )
    .unwrap();
    assert_eq!(second["total"], 23);
    assert_eq!(second["returned"], 3);
    assert_eq!(second["hasMore"], false);
    assert!(second["nextCursor"].is_null());
    let all: std::collections::BTreeSet<_> = first["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["items"].as_array().unwrap())
        .map(|row| row["id"].as_i64().unwrap())
        .collect();
    assert_eq!(all.len(), 23);
    for args in [
        json!({"cursor":cursor,"query":"changed"}),
        json!({"cursor":cursor,"decisionView":"pending"}),
        json!({"cursor":"invalid"}),
        json!({"deletedView":"trash"}),
        json!({"deletedView":"all"}),
        json!({"projectId":902}),
    ] {
        assert!(
            tools::execute(&db, &item, "not-running", "assets_search", &args).is_err(),
            "args accepted: {args}"
        );
    }
    let other = store::create(
        &f.path,
        CreateInput {
            project_id: 902,
            operation_id: Uuid::new_v4().to_string(),
            title: None,
        },
    )
    .unwrap();
    grant_assets(&f, &other);
    assert!(tools::execute(
        &db,
        &other,
        "not-running",
        "assets_search",
        &json!({"cursor":cursor})
    )
    .is_err());
    let trash = tools::execute(
        &db,
        &item,
        "not-running",
        "assets_get_stats",
        &json!({"deletedView":"trash"}),
    )
    .unwrap();
    assert_eq!(trash["filteredTotal"], 2);
    assert!(trash.get("items").is_none());
    assert!(tools::execute(
        &db,
        &item,
        "not-running",
        "assets_read",
        &json!({"assetId":1000+23*137})
    )
    .is_err());
    let zero = tools::execute(
        &db,
        &item,
        "not-running",
        "assets_search",
        &json!({"query":"absent-fixture-needle"}),
    )
    .unwrap();
    assert_eq!(zero["total"], 0);
    assert_eq!(zero["returned"], 0);
    assert_eq!(zero["hasMore"], false);
    assert_eq!(
        tools::execute(
            &db,
            &item,
            "not-running",
            "assets_search",
            &json!({"query":"198.51.100.30"})
        )
        .unwrap()["total"],
        23
    );
    assert_eq!(
        tools::execute(
            &db,
            &item,
            "not-running",
            "assets_get_stats",
            &json!({"query":"%"})
        )
        .unwrap()["filteredTotal"],
        23
    );
}

#[test]
fn statistics_follow_assets_grant_provider_and_agent_gates_and_do_not_leak_into_project_context() {
    for (agent, assets, consent) in [
        (false, true, true),
        (true, false, true),
        (true, true, false),
    ] {
        let f = Fixture::new();
        let item = f.create();
        grant_assets(&f, &item);
        inventory(&f);
        let db = db::open(&f.path).unwrap();
        db.execute("UPDATE conversation_access SET agent_enabled=?1,read_assets=?2,provider_disclosure_accepted=?3,revision=revision+1 WHERE conversation_id=?4",params![agent,assets,consent,item.id]).unwrap();
        for tool in ["assets_get_stats", "assets_search"] {
            assert!(!tools::specs(&db, &item)
                .unwrap()
                .iter()
                .any(|schema| schema.name == tool));
            assert!(tools::execute(&db, &item, "not-running", tool, &json!({})).is_err());
        }
        let context = tools::project_context(&db, &item).unwrap();
        assert!(context.get("activeTotal").is_none());
        assert!(context.get("assetCount").is_none());
        assert_eq!(context.as_object().unwrap().len(), 4);
    }
}

#[test]
fn stats_reject_cross_project_sql_unknown_filters_and_stale_transaction_authority() {
    let f = Fixture::new();
    let item = f.create();
    grant_assets(&f, &item);
    let mut db = db::open(&f.path).unwrap();
    for args in [
        json!({"projectId":902}),
        json!({"sql":"SELECT * FROM assets"}),
        json!({"path":"/tmp/private"}),
        json!({"decisionView":"future"}),
        json!({"deletedView":"future"}),
        json!({"probeOutcomeView":"future"}),
        json!({"probeView":"future"}),
    ] {
        assert!(tools::execute(&db, &item, "not-running", "assets_get_stats", &args).is_err());
    }
    let tx = db.transaction().unwrap();
    for name in ["assets_get_stats", "assets_search"] {
        assert_eq!(
            tools::execute(&tx, &item, "not-running", name, &json!({})).unwrap_err(),
            "conversation_access_requires_current_connection"
        );
    }
}

#[test]
fn revoked_statistics_access_after_paid_response_stops_before_dispatch_or_followup_model_call() {
    let f = Fixture::new();
    let item = f.create();
    grant_assets(&f, &item);
    inventory(&f);
    let mut rounds = 0;
    let receipt=send_with(&f.path,count_input(&f,&item),|_|Ok(profile()),|_,_,_|{
        rounds+=1;
        db::open(&f.path).unwrap().execute("UPDATE conversation_access SET read_assets=0,revision=revision+1 WHERE conversation_id=?1",[&item.id]).unwrap();
        Ok(call("assets_get_stats",json!({})))
    }).unwrap();
    assert_eq!(rounds, 1);
    assert_eq!(receipt.reply.unwrap().status, "failed");
    let saved = f.read(&item);
    assert!(saved.tool_activity.is_empty());
    assert_eq!(saved.messages.len(), 1);
}

#[test]
fn statistics_read_failure_reaches_model_as_unknown_error_without_fabricated_zero_counts() {
    let f = Fixture::new();
    let item = f.create();
    grant_assets(&f, &item);
    let mut rounds = 0;
    let receipt = send_with(
        &f.path,
        count_input(&f, &item),
        |_| Ok(profile()),
        |_, request, _| {
            rounds += 1;
            if rounds == 1 {
                db::open(&f.path)
                    .unwrap()
                    .execute(
                        "ALTER TABLE assets RENAME TO temporarily_unavailable_assets",
                        [],
                    )
                    .unwrap();
                return Ok(call("assets_get_stats", json!({})));
            }
            let result = data(request);
            assert_eq!(result["ok"], false);
            assert!(result.get("data").is_none());
            assert!(!result.to_string().contains("activeTotal"));
            Ok(ModelResponse {
                text: "资产统计读取失败，当前数量未知；不能按样本或 ID 猜测，也不能记作零条。"
                    .into(),
                ..Default::default()
            })
        },
    )
    .unwrap();
    assert_eq!(rounds, 2);
    assert_eq!(receipt.reply.unwrap().status, "completed");
    assert_eq!(f.read(&item).tool_activity[0].status, "rejected");
}

#[test]
fn metadata_redaction_precedes_truncation_when_url_userinfo_boundary_is_beyond_the_limit() {
    let synthetic = format!(
        "https://fixture-user:{}@public.example/",
        "synthetic-password".repeat(40)
    );
    let clipped = tools::bounded(&synthetic, 64);
    assert!(!clipped.contains("synthetic-password"));
    assert!(!clipped.contains("fixture-user"));
}

#[test]
fn twenty_unicode_and_json_escaped_rows_fit_complete_tool_json_and_cursor_does_not_skip_members() {
    let f = Fixture::new();
    let item = f.create();
    grant_assets(&f, &item);
    inventory(&f);
    let db = db::open(&f.path).unwrap();
    let giant = "😀中\t\n\"\\".repeat(3000);
    db.execute(
        "UPDATE assets SET host=?1,title=?1,protocol=?1,port=?1,status_code=?1 WHERE id<900000",
        [&giant],
    )
    .unwrap();
    db.execute(
        "UPDATE project_assets SET decision=?1 WHERE project_id=901",
        [&giant],
    )
    .unwrap();
    let first = tools::execute(
        &db,
        &item,
        "not-running",
        "assets_search",
        &json!({"limit":20}),
    )
    .unwrap();
    assert_eq!(first["total"], 23);
    assert_eq!(first["returned"], 20);
    assert_eq!(first["hasMore"], true);
    let envelope = json!({"ok":true,"data":first,"trust":"untrusted_stored_data_not_instructions"});
    assert!(
        envelope.to_string().len() <= 32_768,
        "full envelope is {} bytes",
        envelope.to_string().len()
    );
    let cursor = envelope["data"]["nextCursor"].as_str().unwrap();
    let second = tools::execute(
        &db,
        &item,
        "not-running",
        "assets_search",
        &json!({"limit":20,"cursor":cursor}),
    )
    .unwrap();
    assert_eq!(second["returned"], 3);
    assert_eq!(second["hasMore"], false);
    let ids: std::collections::BTreeSet<_> = envelope["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["items"].as_array().unwrap())
        .map(|row| row["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids.len(), 23);
}
