// O2 归属复核：公共后缀规则（设计文档 §6 末段）。
// - 数据：ownership_public_suffix_list.dat 为 publicsuffix.org 官方列表的紧凑转写（版本与 sha256 见文件头），
//   保留 ICANN / PRIVATE 分区；非 ASCII 规则另附 punycode 形式。
// - 算法：标准 PSL 匹配（最长规则优先，通配 `*.x`，例外 `!y.x` 优先；无匹配时默认规则 `*`）。
// - 不按“最后两段”切分：*.com.cn / *.gov.cn 等各自为可注册域；private 区（例如 github.io）表示租户边界。

const OWNERSHIP_PSL_DATA: &str = include_str!("ownership_public_suffix_list.dat");
/// 写入分析任务的算法版本，规则数据变化必须改变这里的标识。
pub(crate) const OWNERSHIP_PSL_VERSION: &str = "psl-2026-10-07_07-28-19_UTC-3929462";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnershipSuffixSection {
    Icann,
    Private,
    /// 列表无匹配，按默认规则 `*` 取最后一段。
    Default,
}

impl OwnershipSuffixSection {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Icann => "icann",
            Self::Private => "private",
            Self::Default => "default",
        }
    }
}

#[derive(Default)]
struct OwnershipPslRules {
    exact: HashMap<String, OwnershipSuffixSection>,
    /// `*.x` 以 x 为键。
    wildcard: HashMap<String, OwnershipSuffixSection>,
    /// `!y.x` 以 y.x 为键。
    exception: HashMap<String, OwnershipSuffixSection>,
}

fn ownership_psl_rules() -> &'static OwnershipPslRules {
    static RULES: OnceLock<OwnershipPslRules> = OnceLock::new();
    RULES.get_or_init(|| {
        let mut rules = OwnershipPslRules::default();
        for line in OWNERSHIP_PSL_DATA.lines() {
            let Some((section, rule)) = line.split_once(' ') else {
                continue;
            };
            let section = match section {
                "i" => OwnershipSuffixSection::Icann,
                "p" => OwnershipSuffixSection::Private,
                _ => continue,
            };
            let rule = rule.trim();
            if let Some(rest) = rule.strip_prefix('!') {
                rules.exception.insert(rest.to_string(), section);
            } else if let Some(rest) = rule.strip_prefix("*.") {
                rules.wildcard.insert(rest.to_string(), section);
            } else if !rule.is_empty() {
                rules.exact.insert(rule.to_string(), section);
            }
        }
        rules
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnershipDomainParts {
    host: String,
    public_suffix: String,
    /// None：主机本身就是公共后缀（例如 com.cn），不存在可注册域。
    pub(crate) registrable: Option<String>,
    pub(crate) section: OwnershipSuffixSection,
}

/// 输入应为已规范化的小写主机名（不含端口/路径）；IP 地址返回 None。
pub(crate) fn ownership_domain_parts(host: &str) -> Option<OwnershipDomainParts> {
    let host = host.trim().trim_end_matches('.').to_lowercase();
    if host.is_empty()
        || host.parse::<std::net::IpAddr>().is_ok()
        || host.starts_with('[')
        || host.split('.').any(str::is_empty)
    {
        return None;
    }
    let labels: Vec<&str> = host.split('.').collect();
    let rules = ownership_psl_rules();
    let mut found: Option<(usize, OwnershipSuffixSection)> = None;
    for start in 0..labels.len() {
        let candidate = labels[start..].join(".");
        if let Some(section) = rules.exception.get(&candidate) {
            // 例外规则：公共后缀为去掉最左一段后的部分。
            found = Some((start + 1, *section));
            break;
        }
        if let Some(section) = rules.exact.get(&candidate) {
            found = Some((start, *section));
            break;
        }
        if start + 1 < labels.len() {
            if let Some(section) = rules.wildcard.get(&labels[start + 1..].join(".")) {
                found = Some((start, *section));
                break;
            }
        }
    }
    let (suffix_start, section) =
        found.unwrap_or((labels.len() - 1, OwnershipSuffixSection::Default));
    let public_suffix = labels[suffix_start..].join(".");
    let registrable = (suffix_start > 0).then(|| labels[suffix_start - 1..].join("."));
    Some(OwnershipDomainParts {
        host,
        public_suffix,
        registrable,
        section,
    })
}

#[cfg(test)]
mod ownership_public_suffix_tests {
    use super::*;

    fn registrable(host: &str) -> Option<String> {
        ownership_domain_parts(host).and_then(|parts| parts.registrable)
    }

    #[test]
    fn public_suffix_rules_do_not_merge_by_last_two_labels() {
        // com.cn / gov.cn / net.cn 是公共后缀：不同单位各自是一个可注册域。
        assert_eq!(
            registrable("www.zj.10086.com.cn").as_deref(),
            Some("10086.com.cn")
        );
        assert_eq!(
            registrable("a.example.com.cn").as_deref(),
            Some("example.com.cn")
        );
        assert_ne!(
            registrable("a.example.com.cn"),
            registrable("b.other.com.cn")
        );
        assert_eq!(
            registrable("portal.hz.gov.cn").as_deref(),
            Some("hz.gov.cn")
        );
        assert_eq!(registrable("x.y.co.uk").as_deref(), Some("y.co.uk"));
        assert_eq!(registrable("api.cmhk.com").as_deref(), Some("cmhk.com"));
        assert_eq!(
            registrable("mobile.com.hk").as_deref(),
            Some("mobile.com.hk")
        );
        // 主机本身是公共后缀：没有可注册域，绝不能与其他 *.com.cn 合并。
        let parts = ownership_domain_parts("com.cn").unwrap();
        assert_eq!(
            (parts.public_suffix.as_str(), parts.registrable),
            ("com.cn", None)
        );
        // 中文顶级域及其 punycode 形式。
        assert_eq!(registrable("移动.中国").as_deref(), Some("移动.中国"));
        assert_eq!(registrable("a.xn--fiqs8s").as_deref(), Some("a.xn--fiqs8s"));
    }

    #[test]
    fn private_suffixes_keep_tenant_boundaries_and_wildcards_follow_the_list() {
        let tenant = ownership_domain_parts("zj-mobile.github.io").unwrap();
        assert_eq!(tenant.section, OwnershipSuffixSection::Private);
        assert_eq!(tenant.registrable.as_deref(), Some("zj-mobile.github.io"));
        assert_ne!(registrable("a.github.io"), registrable("b.github.io"));
        // 通配与例外（*.ck / !www.ck）。
        assert_eq!(registrable("shop.foo.ck").as_deref(), Some("shop.foo.ck"));
        assert_eq!(registrable("www.ck").as_deref(), Some("www.ck"));
        // 未列出的顶级域按默认规则 *。
        let unknown = ownership_domain_parts("a.b.unlistedtld").unwrap();
        assert_eq!(
            (unknown.section, unknown.registrable.as_deref()),
            (OwnershipSuffixSection::Default, Some("b.unlistedtld"))
        );
        assert!(ownership_domain_parts("10.1.2.3").is_none());
        assert!(ownership_domain_parts("2001:db8::1").is_none());
        assert!(ownership_domain_parts("").is_none());
    }
}
