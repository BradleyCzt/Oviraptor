// §Q6 — persistent clusters. Grouping is an observation about shared bytes or a shared
// registered domain; it never transfers ownership, authorization or a verified conclusion.
// The representative is chosen deterministically so a re-run produces the same graph.

use super::state::EXPOSURE_RULE_VERSION;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterMember {
    pub asset_id: i64,
    pub origin: String,
    pub content_hash: String,
    /// Highest ownership confidence first, then id, so the representative is the strongest
    /// directly observed member rather than an arbitrary row.
    pub confidence: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterKind {
    /// The same response bytes served from several origins.
    Content,
    /// Names under one registered domain.
    Domain,
}

impl ClusterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::Domain => "domain",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Cluster {
    pub kind: ClusterKind,
    pub cluster_key: String,
    pub representative: i64,
    pub members: Vec<i64>,
    pub rule_version: &'static str,
}

/// G1-1 boundary bases. A domain cluster key says which rule drew its boundary, so a key
/// computed under one PSL snapshot never silently merges with rows from another one.
pub const BOUNDARY_PSL: &str = "psl";
pub const BOUNDARY_EXACT_HOST: &str = "exact_host";
pub const BOUNDARY_CONTENT: &str = "content_hash";
/// Clusters and observations written before G1 carry an empty basis: last-two-labels.
pub const BOUNDARY_LEGACY: &str = "";

/// The PSL snapshot this build groups with, read from the shared ownership module.
pub fn psl_version() -> &'static str {
    crate::commands::OWNERSHIP_PSL_VERSION
}

/// The stored basis of a PSL boundary: `psl:<snapshot>`.
pub fn psl_basis() -> String {
    format!("{BOUNDARY_PSL}:{}", psl_version())
}

/// G1-1: the registrable domain under the shared public-suffix list, read-only through
/// `crate::commands::ownership_domain_parts`. An IP, a host that is itself a public suffix
/// (`github.io`, `com.cn`) or a malformed name has no registrable domain and is grouped by
/// its exact host instead, never by a guessed parent. Returns `(key, basis)`; `basis` is
/// `psl:<snapshot>` or `exact_host`.
pub fn registrable_boundary(host: &str) -> (String, String) {
    let host = host
        .trim()
        .trim_start_matches("*.")
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.is_empty() {
        return (String::new(), BOUNDARY_LEGACY.to_string());
    }
    match crate::commands::ownership_domain_parts(&host).and_then(|parts| parts.registrable) {
        Some(registrable) => (registrable, psl_basis()),
        None => (host, BOUNDARY_EXACT_HOST.to_string()),
    }
}

/// G1-1: the registered domain used for grouping, from the shared PSL (see
/// `registrable_boundary`). G0 used the last two labels, which merged every `*.com.cn`
/// tenant and every `*.github.io` page into one cluster.
pub fn registered_domain(host: &str) -> String {
    registrable_boundary(host).0
}

/// The pre-G1 rule, kept only to label and measure legacy rows: `a.b.corp.invalid` →
/// `corp.invalid`. Not a public-suffix database; never used for a new grouping.
pub fn legacy_last_two_labels(host: &str) -> String {
    let host = host.trim().trim_start_matches("*.").to_ascii_lowercase();
    // An IP address is a name of its own: splitting 10.0.0.8 would merge unrelated hosts that
    // merely share two octets.
    if host.parse::<std::net::IpAddr>().is_ok() {
        return host;
    }
    let parts: Vec<&str> = host.split('.').filter(|part| !part.is_empty()).collect();
    match parts.len() {
        0 => String::new(),
        1 | 2 => host,
        _ => parts[parts.len() - 2..].join("."),
    }
}

/// The host part of a URL or origin, lowercased. §E3: an observation stores this alongside
/// the registered domain so the production cluster pass is one indexed query, not a parse
/// per row.
pub fn host_of(url: &str) -> String {
    let trimmed = url.trim();
    if let Ok(parsed) = reqwest::Url::parse(trimmed) {
        if let Some(host) = parsed.host_str() {
            return host.to_ascii_lowercase();
        }
    }
    // A bare "a.corp.invalid:8443" or "a.corp.invalid/" is still a host to group by.
    if trimmed.parse::<std::net::IpAddr>().is_ok() {
        return trimmed.to_ascii_lowercase();
    }
    trimmed
        .split('/')
        .next()
        .unwrap_or(trimmed)
        .rsplit('@')
        .next()
        .unwrap_or(trimmed)
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// The grouping label for any URL-shaped value: host first, then registered domain.
pub fn domain_of(url: &str) -> String {
    domain_boundary_of(url).0
}

/// G1-1: `(key, basis)` for any URL-shaped value; what an observation stores.
pub fn domain_boundary_of(url: &str) -> (String, String) {
    let host = host_of(url);
    if host.is_empty() {
        return (String::new(), BOUNDARY_LEGACY.to_string());
    }
    registrable_boundary(&host)
}

/// G1-1: what a stored cluster key says about its boundary: `(basis, psl_version)`.
/// `domain:` keys written by this build are PSL boundaries, `host:` keys are exact hosts,
/// `content:` keys are byte groups.
pub fn boundary_of_key(key: &str) -> (String, String) {
    if key.starts_with("domain:") {
        (BOUNDARY_PSL.to_string(), psl_version().to_string())
    } else if key.starts_with("host:") {
        (BOUNDARY_EXACT_HOST.to_string(), String::new())
    } else {
        (BOUNDARY_CONTENT.to_string(), String::new())
    }
}

/// G1-1: one human label per stored basis, including the legacy one.
pub fn boundary_label(kind: &str, basis: &str, psl_version: &str) -> String {
    match (kind, basis) {
        ("content", _) => "相同响应内容".to_string(),
        (_, BOUNDARY_PSL) => format!("公共后缀注册域（{psl_version}）"),
        (_, BOUNDARY_EXACT_HOST) => "精确主机（无可注册域：IP 或本身是公共后缀）".to_string(),
        (_, BOUNDARY_LEGACY) => "旧版末两段分组（G1 之前，未按公共后缀，可能误合并）".to_string(),
        (_, other) => format!("未知边界 {other}"),
    }
}

fn origin_host(origin: &str) -> String {
    let Ok(parsed) = reqwest::Url::parse(origin) else {
        return origin.to_ascii_lowercase();
    };
    parsed.host_str().unwrap_or_default().to_ascii_lowercase()
}

fn pick_representative(rows: &[&ClusterMember]) -> i64 {
    rows.iter()
        .max_by_key(|member| (member.confidence, -member.asset_id))
        .map(|member| member.asset_id)
        .unwrap_or_default()
}

/// Groups by identical response bytes. Singletons are not clusters.
pub fn content_clusters(members: &[ClusterMember]) -> Vec<Cluster> {
    let mut groups: std::collections::BTreeMap<String, Vec<&ClusterMember>> =
        std::collections::BTreeMap::new();
    for member in members {
        if member.content_hash.trim().is_empty() {
            continue;
        }
        groups
            .entry(member.content_hash.clone())
            .or_default()
            .push(member);
    }
    groups
        .into_iter()
        .filter(|(_, rows)| rows.len() > 1)
        .map(|(hash, rows)| {
            let mut ids: Vec<i64> = rows.iter().map(|member| member.asset_id).collect();
            ids.sort_unstable();
            ids.dedup();
            Cluster {
                kind: ClusterKind::Content,
                cluster_key: format!("content:{hash}"),
                representative: pick_representative(&rows),
                members: ids,
                rule_version: EXPOSURE_RULE_VERSION,
            }
        })
        .collect()
}

/// Groups by registered domain. Two hosts on one CDN are *not* one cluster: the shared bytes
/// cluster is the only content grouping, and neither carries ownership.
pub fn domain_clusters(members: &[ClusterMember]) -> Vec<Cluster> {
    let mut groups: std::collections::BTreeMap<String, Vec<&ClusterMember>> =
        std::collections::BTreeMap::new();
    for member in members {
        let (domain, basis) = registrable_boundary(&origin_host(&member.origin));
        if domain.is_empty() {
            continue;
        }
        // G1-1: an exact-host boundary gets its own key space, so `host:github.io` can never
        // collide with a PSL registrable domain of the same spelling.
        let key = if basis == BOUNDARY_EXACT_HOST {
            format!("host:{domain}")
        } else {
            format!("domain:{domain}")
        };
        groups.entry(key).or_default().push(member);
    }
    groups
        .into_iter()
        .filter(|(_, rows)| rows.len() > 1)
        .map(|(key, rows)| {
            let mut ids: Vec<i64> = rows.iter().map(|member| member.asset_id).collect();
            ids.sort_unstable();
            ids.dedup();
            Cluster {
                kind: ClusterKind::Domain,
                cluster_key: key,
                representative: pick_representative(&rows),
                members: ids,
                rule_version: EXPOSURE_RULE_VERSION,
            }
        })
        .collect()
}

/// How a member's conclusion was obtained. Only the representative is a direct observation;
/// everything else is explicitly inherited and says which asset it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inheritance {
    pub asset_id: i64,
    pub inherited_from_asset_id: Option<i64>,
    pub direct: bool,
    pub note: String,
}

pub fn inheritance_for(cluster: &Cluster, asset_id: i64) -> Option<Inheritance> {
    if !cluster.members.contains(&asset_id) {
        return None;
    }
    if asset_id == cluster.representative {
        return Some(Inheritance {
            asset_id,
            inherited_from_asset_id: None,
            direct: true,
            note: "直接观察".to_string(),
        });
    }
    Some(Inheritance {
        asset_id,
        inherited_from_asset_id: Some(cluster.representative),
        direct: false,
        note: format!(
            "继承自资产 {} 的{}簇结论；不代表本资产已被直接验证，也不代表归属或授权",
            cluster.representative,
            cluster.kind.as_str()
        ),
    })
}

/// The two header sets `jobs/native_refinement.rs` writes today. They are the only shapes
/// this adapter accepts, so a renamed column fails loudly instead of quietly producing
/// empty clusters.
pub const CERTIFICATE_CLUSTER_HEADERS: &[&str] = &[
    "company",
    "certificate_key",
    "asset_count",
    "p1_count",
    "p2_count",
    "p3_count",
    "latest_update",
    "status_summary",
    "subject_cn_examples",
    "domain_examples",
    "ip_examples",
    "representative_urls",
    "recommendation",
];
pub const NAME_CLUSTER_HEADERS: &[&str] = &[
    "company",
    "domain_or_ip",
    "asset_count",
    "p1_count",
    "p2_count",
    "p3_count",
    "latest_update",
    "status_summary",
    "subject_cn_examples",
    "domain_examples",
    "ip_examples",
    "representative_urls",
    "recommendation",
];

/// Turns one refinement cluster CSV into grouping input. The CSV has no asset ids, so the
/// caller supplies the resolve step: only rows that already belong to this project's asset
/// ledger become members, and an unresolvable example is counted as `unmapped`, never
/// invented. Nothing here writes: the refinement output stays an observation input.
pub fn members_from_refinement_csv(
    csv_text: &str,
    headers: &[&str],
    resolve: &dyn Fn(&str) -> Option<(i64, String)>,
    unmapped: &mut Vec<String>,
) -> Result<Vec<ClusterMember>, String> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(csv_text.as_bytes());
    let actual: Vec<String> = reader
        .headers()
        .map_err(|error| format!("簇 CSV 表头不可读：{error}"))?
        .iter()
        .map(str::to_string)
        .collect();
    let expected: Vec<String> = headers.iter().map(|field| field.to_string()).collect();
    if actual.first() != expected.first() || !expected.iter().all(|field| actual.contains(field)) {
        return Err(format!(
            "簇 CSV 表头与既定结构不一致：期望 {headers:?}，实际 {actual:?}"
        ));
    }
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|error| format!("簇 CSV 行不可读：{error}"))?;
        let Some(key) = record.get(1) else {
            continue;
        };
        let company = record.get(0).unwrap_or_default().to_string();
        let examples = ["domain_examples", "ip_examples", "representative_urls"]
            .iter()
            .filter_map(|column| {
                actual
                    .iter()
                    .position(|row| row.as_str() == *column)
                    .and_then(|index| record.get(index))
            })
            .flat_map(|value| {
                value
                    .split(';')
                    .map(str::trim)
                    .filter(|row| !row.is_empty())
            })
            .collect::<Vec<_>>();
        let mut owned = 0usize;
        for example in examples {
            match resolve(example) {
                Some((asset_id, origin)) => {
                    rows.push(ClusterMember {
                        asset_id,
                        origin,
                        content_hash: format!("refinement:{company}:{key}"),
                        confidence: 0,
                    });
                    owned += 1;
                }
                None => unmapped.push(format!("{key}={example}")),
            }
        }
        if owned == 0 {
            // A cluster whose examples are all outside this project stays unmapped: it is
            // reported, never used to group assets it does not own.
            unmapped.push(format!("{key}=<全部例外都不在本项目>"));
        }
    }
    Ok(rows)
}

/// Certificates group by the refinement key; the content-hash field carries that key so the
/// resulting cluster is the same shape as a byte-level one.
pub fn clusters_from_refinement(members: &[ClusterMember], kind: ClusterKind) -> Vec<Cluster> {
    let mut groups: std::collections::BTreeMap<String, Vec<&ClusterMember>> =
        std::collections::BTreeMap::new();
    for member in members {
        groups
            .entry(format!("{}:{}", kind.as_str(), member.content_hash))
            .or_default()
            .push(member);
    }
    groups
        .into_iter()
        .map(|(key, rows)| {
            let mut ids: Vec<i64> = rows.iter().map(|member| member.asset_id).collect();
            ids.sort_unstable();
            ids.dedup();
            Cluster {
                kind,
                cluster_key: key,
                representative: *ids.first().unwrap_or(&0),
                members: ids,
                rule_version: EXPOSURE_RULE_VERSION,
            }
        })
        .filter(|cluster: &Cluster| cluster.members.len() > 1)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refinement_text() -> String {
        format!(
            "{}\ncert-sn-1,2,1,1,0,2026-10-01,200:2,CN=corp,a.corp.invalid;b.corp.invalid,,https://a.corp.invalid/;https://b.corp.invalid/,同源证书\n",
            CERTIFICATE_CLUSTER_HEADERS.join(",")
        )
    }

    #[test]
    fn refinement_cluster_csv_becomes_grouping_input_without_inventing_members() {
        let known: std::collections::HashMap<String, (i64, String)> = [
            (
                "https://a.corp.invalid/".to_string(),
                (1, "https://a.corp.invalid/".to_string()),
            ),
            (
                "a.corp.invalid".to_string(),
                (1, "https://a.corp.invalid/".to_string()),
            ),
            (
                "b.corp.invalid".to_string(),
                (2, "https://b.corp.invalid/".to_string()),
            ),
            (
                "https://b.corp.invalid/".to_string(),
                (2, "https://b.corp.invalid/".to_string()),
            ),
        ]
        .into_iter()
        .collect();
        let mut unmapped = Vec::new();
        let members = members_from_refinement_csv(
            &refinement_text(),
            CERTIFICATE_CLUSTER_HEADERS,
            &|value: &str| known.get(value).cloned(),
            &mut unmapped,
        )
        .expect("本项目台账里的例外必须能用");
        let clusters = clusters_from_refinement(&members, ClusterKind::Content);
        assert_eq!(
            clusters.len(),
            1,
            "members={:?} unmapped={:?}",
            members,
            unmapped
        );
        assert_eq!(
            clusters[0].members,
            vec![1, 2],
            "同一证书键的例外必须归成一簇"
        );
        assert_eq!(clusters[0].representative, 1, "代表必须确定");
        // 去重后仍只有两个成员：URL 与裸域名指向同一资产。
        let unique: std::collections::HashSet<i64> =
            members.iter().map(|row| row.asset_id).collect();
        assert_eq!(unique.len(), 2, "{members:?}");

        // 表头一换就报错，不静默产出空簇。
        let broken = refinement_text().replace("certificate_key", "cert_key_v2");
        let error = members_from_refinement_csv(
            &broken,
            CERTIFICATE_CLUSTER_HEADERS,
            &|_| None,
            &mut Vec::new(),
        )
        .expect_err("列名变化必须显式失败");
        assert!(error.contains("表头与既定结构不一致"), "{error}");

        // 例外都不在本项目时只记录 unmapped，不会把别人的名字并进来。
        let mut unmapped = Vec::new();
        let none = members_from_refinement_csv(
            &refinement_text(),
            CERTIFICATE_CLUSTER_HEADERS,
            &|_| None,
            &mut unmapped,
        )
        .expect("解析仍应成功");
        assert!(none.is_empty());
        assert!(
            unmapped
                .iter()
                .any(|row| row.contains("全部例外都不在本项目")),
            "{unmapped:?}"
        );
        assert!(clusters_from_refinement(&none, ClusterKind::Content).is_empty());
    }

    fn member(id: i64, origin: &str, hash: &str, confidence: i64) -> ClusterMember {
        ClusterMember {
            asset_id: id,
            origin: origin.to_string(),
            content_hash: hash.to_string(),
            confidence,
        }
    }

    #[test]
    fn only_repeated_bytes_or_repeated_domains_form_a_cluster() {
        let rows = vec![
            member(1, "https://a.corp.invalid/", "same", 90),
            member(2, "https://b.other.invalid/", "same", 40),
            member(3, "https://c.corp.invalid/", "unique", 80),
            member(4, "https://d.corp.invalid/", "unique2", 80),
        ];
        let content = content_clusters(&rows);
        assert_eq!(content.len(), 1, "{content:?}");
        assert_eq!(content[0].members, vec![1, 2]);
        assert_eq!(content[0].representative, 1, "归属更强的成员才当代表");
        let domains = domain_clusters(&rows);
        assert_eq!(domains.len(), 1, "{domains:?}");
        assert_eq!(domains[0].cluster_key, "domain:corp.invalid");
        assert_eq!(domains[0].members, vec![1, 3, 4]);
    }

    #[test]
    fn inherited_conclusions_are_labelled_and_never_claimed_as_direct() {
        let rows = vec![
            member(1, "https://a.corp.invalid/", "same", 90),
            member(2, "https://b.corp.invalid/", "same", 50),
        ];
        let cluster = content_clusters(&rows).remove(0);
        let direct = inheritance_for(&cluster, 1).expect("代表必须在簇里");
        assert!(direct.direct);
        assert_eq!(direct.inherited_from_asset_id, None);
        let inherited = inheritance_for(&cluster, 2).expect("成员必须在簇里");
        assert!(!inherited.direct);
        assert_eq!(inherited.inherited_from_asset_id, Some(1));
        assert!(
            inherited.note.contains("不代表归属或授权"),
            "{}",
            inherited.note
        );
        assert!(inheritance_for(&cluster, 99).is_none());
    }

    #[test]
    fn a_shared_name_or_host_does_not_merge_ownership() {
        assert_eq!(registered_domain("*.API.Corp.Invalid"), "corp.invalid");
        assert_eq!(registered_domain("10.0.0.8"), "10.0.0.8");
        assert_eq!(registered_domain("::1"), "::1");
        // 同一个 CDN 内容簇里的两个域名簇彼此独立：域名不同就不合并。
        let rows = vec![
            member(1, "https://cdn-a.invalid/x", "same", 90),
            member(2, "https://cdn-b.invalid/y", "same", 90),
        ];
        let content = content_clusters(&rows);
        assert_eq!(content[0].members, vec![1, 2]);
        assert!(domain_clusters(&rows).is_empty(), "不同注册域不能拼成一簇");
    }
}
