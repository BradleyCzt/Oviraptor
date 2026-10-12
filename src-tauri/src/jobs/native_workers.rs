use base64::Engine;
use csv::{ReaderBuilder, WriterBuilder};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    io::{BufRead, Read, Write},
    net::{IpAddr, TcpStream, ToSocketAddrs},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

type Record = HashMap<String, String>;

#[path = "native_content.rs"]
mod native_content;
#[path = "native_probe_control.rs"]
mod native_probe_control;

fn read_csv(path: &Path) -> Result<(Vec<String>, Vec<Record>), String> {
    let mut reader = ReaderBuilder::new()
        .flexible(true)
        .from_path(path)
        .map_err(|error| error.to_string())?;
    let fields = reader
        .headers()
        .map_err(|error| error.to_string())?
        .iter()
        .enumerate()
        .map(|(index, value)| {
            if index == 0 {
                value.trim_start_matches('\u{feff}').to_string()
            } else {
                value.to_string()
            }
        })
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|error| error.to_string())?;
        rows.push(
            fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    (
                        field.clone(),
                        record.get(index).unwrap_or_default().trim().to_string(),
                    )
                })
                .collect(),
        );
    }
    Ok((fields, rows))
}
fn text(row: &Record, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|k| {
            row.get(*k)
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_default()
}
fn endpoint(row: &Record) -> String {
    let raw = text(row, &["probe_effective_url", "link", "host"]);
    let parsed = reqwest::Url::parse(&raw)
        .or_else(|_| reqwest::Url::parse(&format!("http://{raw}")))
        .ok();
    let mut host = parsed
        .as_ref()
        .and_then(reqwest::Url::host_str)
        .unwrap_or_else(|| {
            row.get("domain")
                .or_else(|| row.get("ip"))
                .map(String::as_str)
                .unwrap_or("")
        })
        .trim_end_matches('.')
        .to_lowercase();
    if let Some(v) = host.strip_prefix("www.") {
        host = v.into()
    }
    let path = parsed
        .as_ref()
        .map(reqwest::Url::path)
        .unwrap_or("/")
        .trim_end_matches('/');
    format!(
        "{}|{}|{}",
        text(row, &["company"]).to_lowercase(),
        host,
        if path.is_empty() { "/" } else { path }
    )
}
fn number(row: &Record, keys: &[&str]) -> i64 {
    text(row, keys).parse().unwrap_or(0)
}
fn rank(row: &Record) -> (i32, i32, bool, bool) {
    let status = number(row, &["probe_status_code", "status_code"]);
    let outcome = text(row, &["probe_outcome"]);
    let url = text(row, &["probe_effective_url", "link", "host"]);
    let sr = if (200..300).contains(&status) {
        5
    } else if (300..400).contains(&status) {
        4
    } else if matches!(status, 401 | 403) {
        3
    } else if (400..500).contains(&status) {
        2
    } else {
        0
    };
    let or = match outcome.as_str() {
        "web_alive" => 6,
        "virtual_host_required" => 5,
        "web_restricted" => 4,
        "browser_render_required" => 3,
        "tcp_alive_non_http" => 2,
        "alive_clean" => 1,
        _ => 0,
    };
    (
        or,
        sr,
        url.starts_with("https://"),
        !text(row, &["probe_title", "title"]).is_empty(),
    )
}
fn reason(row: &Record) -> String {
    let status = number(row, &["probe_status_code", "status_code"]);
    match text(row, &["probe_outcome"]).as_str() {
        "web_abnormal" => {
            let v = text(row, &["probe_entry_state"]);
            format!(
                "web_abnormal:{}",
                if v.is_empty() {
                    format!("HTTP_{status}")
                } else {
                    v
                }
            )
        }
        "unreachable" => {
            let v = text(row, &["probe_entry_state"]);
            format!(
                "unreachable:{}",
                if v.is_empty() {
                    "connect_failed".into()
                } else {
                    v
                }
            )
        }
        "skipped" => {
            let v = text(row, &["probe_error"]);
            format!(
                "skipped:{}",
                if v.is_empty() {
                    "invalid_target".into()
                } else {
                    v
                }
            )
        }
        _ => String::new(),
    }
}
fn write_csv(path: &Path, fields: &[String], rows: &[Record]) -> Result<(), String> {
    let mut w = WriterBuilder::new()
        .from_path(path)
        .map_err(|e| e.to_string())?;
    w.write_record(fields).map_err(|e| e.to_string())?;
    for row in rows {
        w.write_record(
            fields
                .iter()
                .map(|f| row.get(f).map(String::as_str).unwrap_or("")),
        )
        .map_err(|e| e.to_string())?
    }
    w.flush().map_err(|e| e.to_string())
}

const ACTIVE_STATUSES: &[&str] = &[
    "200", "201", "202", "204", "206", "301", "302", "303", "307", "308", "401", "403",
];

include!("native_refinement.rs");

include!("native_probe.rs");

include!("native_fofa.rs");

pub fn merge_url_inventory(input: &Path, output: &Path) -> Result<serde_json::Value, String> {
    let (_, rows) = read_csv(input)?;
    let mut best = HashMap::<String, Record>::new();
    for mut row in rows {
        let raw = text(&row, &["link", "host", "ip"]);
        if raw.is_empty() {
            continue;
        }
        let mut url = if raw.contains("://") {
            raw
        } else {
            let protocol = text(&row, &["protocol"]).to_ascii_lowercase();
            let port = number(&row, &["port"]);
            let scheme = if protocol.contains("https") || port == 443 {
                "https"
            } else {
                "http"
            };
            let mut parsed = reqwest::Url::parse(&format!("{scheme}://{raw}")).ok();
            if let Some(parsed) = parsed.as_mut() {
                if parsed.port().is_none()
                    && port > 0
                    && !((scheme == "http" && port == 80) || (scheme == "https" && port == 443))
                {
                    let _ = parsed.set_port(Some(port as u16));
                }
            }
            parsed
                .map(|value| value.to_string())
                .unwrap_or_else(|| format!("{scheme}://{raw}"))
        };
        if let Ok(mut parsed) = reqwest::Url::parse(&url) {
            parsed.set_fragment(None);
            url = parsed.to_string().trim_end_matches('/').to_string();
        }
        let decision = text(&row, &["decision", "人工结论", "结论"]).to_ascii_lowercase();
        let bucket = if [
            "confirmed",
            "confirm",
            "yes",
            "true",
            "1",
            "确认",
            "已确认",
            "是",
        ]
        .contains(&decision.as_str())
        {
            "confirmed"
        } else if ["rejected", "reject", "no", "false", "0", "排除", "否"]
            .contains(&decision.as_str())
        {
            "rejected"
        } else {
            "pending"
        };
        row.insert("normalized_url".into(), url.clone());
        row.insert("inventory_bucket".into(), bucket.into());
        let replace = best
            .get(&url)
            .map(|old| number(&row, &["score"]) > number(old, &["score"]))
            .unwrap_or(true);
        if replace {
            best.insert(url, row);
        }
    }
    let mut merged = best.into_values().collect::<Vec<_>>();
    merged.sort_by(|left, right| {
        text(left, &["normalized_url"]).cmp(&text(right, &["normalized_url"]))
    });
    let mut fields = vec!["normalized_url".into(), "inventory_bucket".into()];
    for row in &merged {
        for field in row.keys() {
            if !fields.contains(field) {
                fields.push(field.clone());
            }
        }
    }
    write_csv(output, &fields, &merged)?;
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    for (bucket, name) in [
        ("confirmed", "confirmed_urls.txt"),
        ("pending", "pending_urls.txt"),
        ("rejected", "rejected_urls.txt"),
    ] {
        let values = merged
            .iter()
            .filter(|row| text(row, &["inventory_bucket"]) == bucket)
            .map(|row| text(row, &["normalized_url"]))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(
            parent.join(name),
            if values.is_empty() {
                values
            } else {
                format!("{values}\n")
            },
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(
        json!({"input": merged.len(), "unique": merged.len(), "confirmed": merged.iter().filter(|row| text(row, &["inventory_bucket"]) == "confirmed").count(), "pending": merged.iter().filter(|row| text(row, &["inventory_bucket"]) == "pending").count(), "rejected": merged.iter().filter(|row| text(row, &["inventory_bucket"]) == "rejected").count()}),
    )
}
pub fn optimize_src_assets(input: &Path, output: &Path) -> Result<serde_json::Value, String> {
    fs::create_dir_all(output).map_err(|e| e.to_string())?;
    let accepted = [
        "web_alive",
        "web_restricted",
        "browser_render_required",
        "virtual_host_required",
        "web_abnormal",
        "tcp_alive_non_http",
        "alive_clean",
        "blocked_content",
        "unreachable",
        "skipped",
    ];
    let mut paths = fs::read_dir(input)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|v| v.to_str()) == Some("csv")
                && accepted.iter().any(|v| {
                    p.file_name()
                        .and_then(|v| v.to_str())
                        .unwrap_or("")
                        .contains(v)
                })
        })
        .collect::<Vec<_>>();
    paths.sort();
    let (mut rows, mut fields) = (Vec::<Record>::new(), Vec::<String>::new());
    for path in paths {
        let mut r = ReaderBuilder::new()
            .flexible(true)
            .from_path(path)
            .map_err(|e| e.to_string())?;
        let h = r
            .headers()
            .map_err(|e| e.to_string())?
            .iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        for v in &h {
            if !fields.contains(v) {
                fields.push(v.clone())
            }
        }
        for rec in r.records() {
            let rec = rec.map_err(|e| e.to_string())?;
            rows.push(
                h.iter()
                    .enumerate()
                    .map(|(i, k)| (k.clone(), rec.get(i).unwrap_or("").into()))
                    .collect(),
            )
        }
    }
    let mut best = HashMap::<String, usize>::new();
    for (i, row) in rows.iter().enumerate() {
        if !reason(row).is_empty() {
            continue;
        }
        let key = endpoint(row);
        if best
            .get(&key)
            .map(|old| rank(row) > rank(&rows[*old]))
            .unwrap_or(true)
        {
            best.insert(key, i);
        }
    }
    let (mut keep, mut excluded) = (Vec::new(), Vec::new());
    for (i, row) in rows.iter().enumerate() {
        let mut why = reason(row);
        if why.is_empty() && best.get(&endpoint(row)).copied() != Some(i) {
            why = "duplicate_endpoint".into()
        }
        let mut out = row.clone();
        if why.is_empty() {
            keep.push(out)
        } else {
            out.insert("auto_excluded".into(), "1".into());
            out.insert("exclude_reason".into(), why);
            excluded.push(out)
        }
    }
    for v in ["auto_excluded", "exclude_reason"] {
        if !fields.iter().any(|x| x == v) {
            fields.push(v.into())
        }
    }
    write_csv(&output.join("optimized_assets.csv"), &fields, &keep)?;
    write_csv(&output.join("auto_excluded.csv"), &fields, &excluded)?;
    let summary = json!({"input":rows.len(),"kept":keep.len(),"excluded":excluded.len(),"web_alive":rows.iter().filter(|r|matches!(text(r,&["probe_outcome"]).as_str(),"web_alive"|"web_restricted"|"browser_render_required"|"virtual_host_required")).count(),"tcp_non_web":rows.iter().filter(|r|text(r,&["probe_outcome"])=="tcp_alive_non_http").count(),"web_abnormal":rows.iter().filter(|r|text(r,&["probe_outcome"])=="web_abnormal").count(),"unreachable":rows.iter().filter(|r|reason(r).starts_with("unreachable:")).count()});
    fs::write(
        output.join("summary.json"),
        serde_json::to_vec_pretty(&summary).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(summary)
}
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn fofa_plan_preserves_profile_fields_and_domain_boundaries() {
        assert!(host_matches_scope("APP.EXAMPLE.TEST.", "example.test"));
        assert!(!host_matches_scope("notexample.test", "example.test"));
        assert!(!host_matches_scope(
            "example.test.evil.test",
            "example.test"
        ));
        assert!(!host_matches_scope("anything", ""));
        let basic = fofa_fields("basic");
        let personal = fofa_fields("personal");
        let corporate = fofa_fields("corporate");
        assert!(basic.len() < personal.len());
        assert!(personal.len() < corporate.len());
        assert!(corporate.iter().any(|field| field == "fid"));
        assert!(!basic.iter().any(|field| field == "fid"));
        assert_eq!(
            corporate.iter().collect::<HashSet<_>>().len(),
            corporate.len()
        );
    }

    #[test]
    fn optimizer_matches_python_contract() {
        let root = std::env::temp_dir().join(format!("oviraptor-native-opt-{}", Uuid::new_v4()));
        let input = root.join("in");
        let output = root.join("out");
        fs::create_dir_all(&input).unwrap();
        fs::write(input.join("P1_web_alive.csv"), "company,probe_effective_url,probe_outcome,probe_status_code,probe_title\nAcme,http://example.com,web_alive,200,Home\nAcme,https://example.com,web_alive,200,Home\nAcme,https://bad.example.com,unreachable,0,\n").unwrap();
        let summary = optimize_src_assets(&input, &output).unwrap();
        assert_eq!(summary["kept"], 1);
        assert_eq!(summary["excluded"], 2);
        assert!(fs::read_to_string(output.join("optimized_assets.csv"))
            .unwrap()
            .contains("https://example.com"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn refinement_preserves_tiers_and_companion_files() {
        let root = std::env::temp_dir().join(format!("oviraptor-native-refine-{}", Uuid::new_v4()));
        let input = root.join("candidates.csv");
        let query_log = root.join("query_log.csv");
        let output = root.join("refined");
        fs::create_dir_all(&root).unwrap();
        fs::write(&input, "company,asset_key,link,domain,ip,protocol,status_code,lastupdatetime,evidence,cert.subject.org,cert.subject.cn,cert.sn\nAcme,one,https://a.example,a.example,1.1.1.1,https,200,2026-08-01,精确主机:a.example,,,,\nAcme,two,https://b.example,b.example,2.2.2.2,https,500,2026-08-01,标题全称:Acme,,,,\n").unwrap();
        fs::write(
            &query_log,
            "status,category,query,total_matches\nerror,seed_domain,domain=example,0\n",
        )
        .unwrap();
        let summary = refine_candidates(&input, &query_log, &output, "2025-01-01").unwrap();
        assert_eq!(summary["source_rows"], 2);
        assert_eq!(summary["priority_rows"], 1);
        assert!(fs::read_to_string(output.join("P1_active_strong.csv"))
            .unwrap()
            .contains("P1_强归属且近期活跃"));
        assert!(output.join("certificate_clusters.csv").exists());
        assert!(output.join("retry_plan.csv").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fofa_seed_plan_is_native_and_deduplicated() {
        let root = std::env::temp_dir().join(format!("oviraptor-native-fofa-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let seeds = root.join("seeds.csv");
        fs::write(&seeds, "company,names,aliases,domains,icps,ip_ranges,asn_orgs,keywords\nAcme,Acme,,https://app.example.com,ICP-1,8.8.8.8,AS-Acme,UniquePortal\n").unwrap();
        let queries = build_fofa_queries(&seeds, true).unwrap();
        assert!(queries
            .iter()
            .any(|query| query.query == "host=\"app.example.com\""));
        assert!(queries
            .iter()
            .any(|query| query.query == "ip=\"8.8.8.0/24\""));
        assert_eq!(
            queries
                .iter()
                .map(|query| (&query.company, &query.category, &query.query))
                .collect::<HashSet<_>>()
                .len(),
            queries.len()
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_probe_writes_the_same_outcome_contract() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 2048];
            let _ = stream.read(&mut buffer);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 45\r\nConnection: close\r\n\r\n<html><title>Native Probe</title>Hello</html>").unwrap();
        });
        let root = std::env::temp_dir().join(format!("oviraptor-native-probe-{}", Uuid::new_v4()));
        let refined = root.join("refined");
        let output = root.join("probe");
        fs::create_dir_all(&refined).unwrap();
        let header = "company,asset_key,link,host,ip,port,protocol\n";
        fs::write(
            refined.join("P1_active_strong.csv"),
            format!(
                "{header}Acme,one,http://{address},127.0.0.1,127.0.0.1,{},http\n",
                address.port()
            ),
        )
        .unwrap();
        fs::write(refined.join("P2_strong_needs_validation.csv"), header).unwrap();
        fs::write(refined.join("P3_name_candidates.csv"), header).unwrap();
        let options = ProbeOptions {
            workers: 2,
            timeout: Duration::from_secs(2),
            retries: 0,
            max_body_bytes: 4096,
            include_other: false,
            include_weak: false,
            allow_private: true,
            strict_tls: false,
            scheme_fallback: false,
            content_threshold: 12,
            replace_default_content_rules: false,
            gambling_keywords: Vec::new(),
            porn_keywords: Vec::new(),
            negative_keywords: Vec::new(),
            custom_keywords: Vec::new(),
            priority_rate: 1000.0,
            other_rate: 1000.0,
            per_host_interval: Duration::ZERO,
        };
        let cancel = AtomicBool::new(false);
        probe_assets(
            &refined,
            &refined.join("P1_active_strong.csv"),
            &output,
            options.clone(),
            &cancel,
            |_, _| {},
        )
        .unwrap();
        server.join().unwrap();
        let alive = fs::read_to_string(output.join("P1_web_alive.csv")).unwrap();
        assert!(alive.contains("Native Probe"));
        assert!(alive.contains("web_alive"));
        // The listener has closed: the second pass can succeed only by using
        // this exact input/configuration's persisted results.
        let resumed = probe_assets(
            &refined,
            &refined.join("P1_active_strong.csv"),
            &output,
            options.clone(),
            &cancel,
            |_, _| {},
        )
        .unwrap();
        assert_eq!(resumed["reused_results"], 1);
        let mut changed = options;
        changed.content_threshold = 99;
        let refreshed = probe_assets(
            &refined,
            &refined.join("P1_active_strong.csv"),
            &output,
            changed,
            &cancel,
            |_, _| {},
        )
        .unwrap();
        assert_eq!(refreshed["reused_results"], 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_url_inventory_keeps_best_decision_record() {
        let root = std::env::temp_dir().join(format!("oviraptor-native-merge-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let input = root.join("input.csv");
        fs::write(&input, "link,protocol,port,score,decision\nexample.com,https,443,20,\nhttps://example.com/,https,443,90,confirmed\n").unwrap();
        let output = root.join("inventory.csv");
        let summary = merge_url_inventory(&input, &output).unwrap();
        assert_eq!(summary["unique"], 1);
        assert_eq!(summary["confirmed"], 1);
        assert!(fs::read_to_string(root.join("confirmed_urls.txt"))
            .unwrap()
            .contains("https://example.com"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_probe_cancel_then_resume_keeps_only_finished_results() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let mut served = 0;
            while served < 2 && std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // macOS/BSD: an accepted socket inherits the listener's
                        // O_NONBLOCK, so a single read returned WouldBlock at once
                        // (ignoring the timeout) and the reply + close raced the
                        // client's request; unread request bytes then reset the
                        // connection ("error sending request") under load. Read the
                        // whole request head in blocking mode, like test_http_endpoint.
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut request = Vec::new();
                        let mut input = [0; 4096];
                        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                            match stream.read(&mut input) {
                                Ok(0) | Err(_) => break,
                                Ok(n) => request.extend_from_slice(&input[..n]),
                            }
                        }
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
                        served += 1;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("fixture server: {error}"),
                }
            }
            served
        });
        let root = std::env::temp_dir().join(format!("oviraptor-native-resume-{}", Uuid::new_v4()));
        let refined = root.join("refined");
        let output = root.join("probe");
        fs::create_dir_all(&refined).unwrap();
        let input = refined.join("P1_active_strong.csv");
        for name in ["P2_strong_needs_validation.csv", "P3_name_candidates.csv"] {
            fs::write(
                refined.join(name),
                "company,asset_key,link,host,ip,port,protocol\n",
            )
            .unwrap();
        }
        fs::write(&input, format!("company,asset_key,link,host,ip,port,protocol\nTest,one,http://{address}/one,127.0.0.1,127.0.0.1,{},http\nTest,two,http://{address}/two,127.0.0.1,127.0.0.1,{},http\n", address.port(), address.port())).unwrap();
        let options = ProbeOptions {
            workers: 1,
            timeout: Duration::from_secs(2),
            retries: 0,
            max_body_bytes: 4096,
            include_other: false,
            include_weak: false,
            allow_private: true,
            strict_tls: false,
            scheme_fallback: false,
            content_threshold: 12,
            replace_default_content_rules: false,
            gambling_keywords: vec![],
            porn_keywords: vec![],
            negative_keywords: vec![],
            custom_keywords: vec![],
            priority_rate: 1000.0,
            other_rate: 1000.0,
            per_host_interval: Duration::ZERO,
        };
        let cancel = AtomicBool::new(false);
        let first = probe_assets(
            &refined,
            &input,
            &output,
            options.clone(),
            &cancel,
            |done, _| {
                if done == 1 {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
        );
        assert_eq!(first.unwrap_err(), "__CANCELLED__");
        let journal = fs::read_dir(&output)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension().is_some_and(|v| v == "jsonl"))
            .unwrap();
        // Simulate a torn final write after a process crash.
        fs::OpenOptions::new()
            .append(true)
            .open(&journal)
            .unwrap()
            .write_all(b"{\"stage\":")
            .unwrap();
        cancel.store(false, Ordering::Relaxed);
        let result = probe_assets(&refined, &input, &output, options, &cancel, |_, _| {}).unwrap();
        assert_eq!(result["reused_results"], 1);
        assert_eq!(server.join().unwrap(), 2);
        let (_, rows) = read_csv(&output.join("P1_web_alive.csv")).unwrap();
        assert_eq!(rows.len(), 2);
        let _ = fs::remove_dir_all(root);
    }
}
