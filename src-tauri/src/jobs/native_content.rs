//! Content classification compatible with the former worker's weighted rules.
//! Compile once per job, never once per response.
use super::ProbeOptions;
use regex::{Regex, RegexBuilder};
use std::collections::HashSet;

const GAMBLING: &[(&str, i64)] = &[
    ("在线赌博", 8),
    ("网络赌博", 7),
    ("赌博平台", 9),
    ("博彩平台", 8),
    ("在线博彩", 8),
    ("真人视讯", 9),
    ("真人娱乐", 7),
    ("体育投注", 8),
    ("彩票下注", 9),
    ("在线娱乐城", 8),
    ("澳门赌场", 8),
    ("百家乐", 6),
    ("送彩金", 5),
    ("首存优惠", 6),
    ("代理返佣", 5),
    (r"\bcasino\b", 7),
    (r"\bsportsbook\b", 8),
    (r"\bbetting\b", 6),
    (r"\bbaccarat\b", 7),
    (r"\broulette\b", 6),
    (r"\bslot\s*games?\b", 6),
];
const PORN: &[(&str, i64)] = &[
    ("色情网站", 10),
    ("成人网站", 10),
    ("成人视频", 9),
    ("成人影片", 9),
    ("无码视频", 9),
    ("无码中文字幕", 8),
    ("激情视频", 8),
    ("情色直播", 9),
    ("约炮", 10),
    ("裸聊", 9),
    ("av女优", 8),
    ("福利姬", 7),
    (r"\bporn(?:hub)?\b", 10),
    (r"\bporno\b", 10),
    (r"\bxxx\b", 7),
    (r"\bhentai\b", 8),
    (r"\bxvideos\b", 10),
    (r"\bxnxx\b", 10),
    (r"\badult\s*(?:video|movie|live|dating|content)s?\b", 8),
    (r"\bsex\s*(?:cam|video|movie|chat)s?\b", 9),
];
const GAMBLING_NEGATIVE: &[&str] = &[
    "打击赌博",
    "禁止赌博",
    "远离赌博",
    "赌博危害",
    "赌博治理",
    "反诈",
    "公安",
    "法院",
    "检察院",
    "举报",
    "专项整治",
    "普法",
    "风险监测",
    "新闻报道",
    "百科",
];
const PORN_NEGATIVE: &[&str] = &[
    "扫黄打非",
    "打击色情",
    "禁止色情",
    "举报色情",
    "色情治理",
    "公安",
    "法院",
    "检察院",
    "健康教育",
    "专项整治",
    "新闻报道",
    "百科",
];

struct WeightedPattern {
    expression: Regex,
    weight: i64,
}

pub(super) struct ContentRules {
    gambling: Vec<WeightedPattern>,
    porn: Vec<WeightedPattern>,
    custom: Vec<WeightedPattern>,
    gambling_negative: Vec<String>,
    porn_negative: Vec<String>,
}

fn compile(
    defaults: &[(&str, i64)],
    custom: &[String],
    weight: i64,
    ignore_numeric: bool,
) -> Vec<WeightedPattern> {
    let mut patterns = defaults
        .iter()
        .map(|(p, w)| (p.to_string(), *w))
        .collect::<Vec<_>>();
    patterns.extend(
        custom
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty() && (!ignore_numeric || !s.chars().all(char::is_numeric)))
            .map(|s| (regex::escape(s), weight)),
    );
    let mut seen = HashSet::new();
    patterns
        .into_iter()
        .filter(|(pattern, _)| seen.insert(pattern.clone()))
        .map(|(pattern, weight)| WeightedPattern {
            expression: RegexBuilder::new(&pattern)
                .case_insensitive(true)
                .build()
                .expect("validated content expression"),
            weight,
        })
        .collect()
}

fn score(
    patterns: &[WeightedPattern],
    negatives: &[String],
    url: &str,
    title: &str,
    body: &str,
) -> (i64, Vec<String>) {
    let mut score = 0;
    let mut matches = Vec::new();
    for pattern in patterns {
        for (name, input, multiplier) in [("title", title, 2), ("url", url, 2), ("body", body, 1)] {
            let hits = pattern.expression.find_iter(input).take(2).count() as i64;
            if hits > 0 {
                score += pattern.weight * if name == "body" { hits } else { multiplier };
                matches.push(format!("{name}:{}", pattern.expression.as_str()));
            }
        }
    }
    for marker in negatives {
        if title.contains(marker) {
            score -= 12;
            matches.push(format!("negative-title:{marker}"));
        } else if body.contains(marker) {
            score -= 3;
            matches.push(format!("negative-body:{marker}"));
        }
    }
    (score.max(0), matches)
}

impl ContentRules {
    pub(super) fn from_options(options: &ProbeOptions) -> Self {
        let negatives = |defaults: &[&str]| {
            let mut values = if options.replace_default_content_rules {
                Vec::new()
            } else {
                defaults.iter().map(|v| v.to_string()).collect::<Vec<_>>()
            };
            values.extend(
                options
                    .negative_keywords
                    .iter()
                    .filter(|v| !v.trim().is_empty())
                    .map(|v| v.trim().to_string()),
            );
            let mut seen = HashSet::new();
            values.retain(|v| seen.insert(v.clone()));
            values
        };
        Self {
            gambling: compile(
                if options.replace_default_content_rules {
                    &[]
                } else {
                    GAMBLING
                },
                &options.gambling_keywords,
                8,
                true,
            ),
            porn: compile(
                if options.replace_default_content_rules {
                    &[]
                } else {
                    PORN
                },
                &options.porn_keywords,
                9,
                true,
            ),
            custom: compile(&[], &options.custom_keywords, 12, false),
            gambling_negative: negatives(GAMBLING_NEGATIVE),
            porn_negative: negatives(PORN_NEGATIVE),
        }
    }

    pub(super) fn classify(
        &self,
        url: &str,
        title: &str,
        body: &str,
        threshold: i64,
    ) -> (String, i64, String) {
        let (url, title, body) = (
            url.to_lowercase(),
            title.to_lowercase(),
            body.to_lowercase(),
        );
        let finish = |category: &str, score: i64, evidence: Vec<String>| {
            (
                category.into(),
                score,
                evidence
                    .into_iter()
                    .take(20)
                    .collect::<Vec<_>>()
                    .join(" | "),
            )
        };
        let (custom, evidence) = score(&self.custom, &[], &url, &title, &body);
        if custom >= threshold {
            return finish("custom_rule", custom, evidence);
        }
        let (gambling, g_evidence) =
            score(&self.gambling, &self.gambling_negative, &url, &title, &body);
        let (porn, p_evidence) = score(&self.porn, &self.porn_negative, &url, &title, &body);
        if gambling >= porn && gambling >= threshold {
            return finish("gambling", gambling, g_evidence);
        }
        if porn >= threshold {
            return finish("porn", porn, p_evidence);
        }
        if gambling >= porn {
            finish("clean", gambling, g_evidence)
        } else {
            finish("clean", porn, p_evidence)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options() -> ProbeOptions {
        ProbeOptions {
            workers: 1,
            timeout: std::time::Duration::from_secs(1),
            total_timeout: std::time::Duration::from_secs(8),
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
            per_host_interval: std::time::Duration::ZERO,
        }
    }

    #[test]
    fn preserves_weighted_title_url_and_body_contract() {
        let rules = ContentRules::from_options(&options());
        let (category, score, evidence) = rules.classify(
            "https://fixture.test/casino",
            "Casino",
            "casino casino casino",
            12,
        );
        assert_eq!(category, "gambling");
        assert_eq!(score, 42);
        assert_eq!(
            evidence,
            r"title:\bcasino\b | url:\bcasino\b | body:\bcasino\b"
        );
    }

    #[test]
    fn preserves_negative_context_and_custom_rule_category() {
        let mut options = options();
        options.custom_keywords = vec!["internal-fixture".into()];
        let rules = ContentRules::from_options(&options);
        assert_eq!(
            rules
                .classify("https://fixture.test", "公安 新闻报道 百科", "在线赌博", 12)
                .0,
            "clean"
        );
        let result = rules.classify("https://fixture.test", "", "internal-fixture", 12);
        assert_eq!((result.0.as_str(), result.1), ("custom_rule", 12));
    }

    #[test]
    fn replacement_rules_ignore_numeric_fragments() {
        let mut options = options();
        options.replace_default_content_rules = true;
        options.porn_keywords = vec!["91".into()];
        let rules = ContentRules::from_options(&options);
        assert_eq!(
            rules
                .classify("https://fixture.test/91", "91", "91 casino casino", 12)
                .1,
            0
        );
    }
}
