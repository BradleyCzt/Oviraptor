//! Skill document patching and explicit refinement policy.
//! The caller supplies model transport; this module owns no credentials.

use serde_json::Value as JsonValue;
use std::collections::HashSet;

fn patch_section(value: &JsonValue) -> Option<(String, String)> {
    let (title, body) = if let Some(text) = value.as_str() {
        let mut lines = text.lines();
        let first = lines.next().unwrap_or("").trim();
        if first.starts_with('#') {
            (
                first.trim_start_matches('#').trim().to_string(),
                lines.collect::<Vec<_>>().join("\n").trim().to_string(),
            )
        } else {
            ("学习补丁".to_string(), format!("- {}", text.trim()))
        }
    } else {
        let object = value.as_object()?;
        let title = object
            .get("title")
            .or(object.get("name"))
            .and_then(JsonValue::as_str)
            .unwrap_or("学习补丁")
            .trim()
            .to_string();
        let body = object
            .get("content")
            .or_else(|| object.get("instructions"))
            .or_else(|| object.get("body"))
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        (title, body)
    };
    if title.is_empty() || body.is_empty() {
        None
    } else {
        Some((title, body))
    }
}

fn normalize_section_title(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('#')
        .trim()
        .to_ascii_lowercase()
}

fn parse_skill_sections(instructions: &str) -> (String, Vec<(String, String)>) {
    let mut preamble = Vec::new();
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut current: Option<(String, Vec<String>)> = None;
    for line in instructions.lines() {
        let trimmed = line.trim();
        let is_heading = trimmed.starts_with("## ") || trimmed.starts_with("### ");
        if is_heading {
            if let Some((title, body)) = current.take() {
                sections.push((title, body.join("\n").trim().to_string()));
            }
            current = Some((
                trimmed.trim_start_matches('#').trim().to_string(),
                Vec::new(),
            ));
        } else if let Some((_, body)) = current.as_mut() {
            body.push(line.to_string());
        } else {
            preamble.push(line.to_string());
        }
    }
    if let Some((title, body)) = current {
        sections.push((title, body.join("\n").trim().to_string()));
    }
    (preamble.join("\n").trim().to_string(), sections)
}

pub(crate) fn apply_skill_patch(base: &str, patch: &JsonValue) -> String {
    let (preamble, mut sections) = parse_skill_sections(base);
    let mut add_without_heading = Vec::new();

    if let Some(items) = patch.get("replaceSections").and_then(JsonValue::as_array) {
        for item in items {
            if let Some((title, body)) = patch_section(item) {
                let key = normalize_section_title(&title);
                if let Some(existing) = sections
                    .iter_mut()
                    .find(|(old_title, _)| normalize_section_title(old_title) == key)
                {
                    existing.1 = body;
                } else {
                    sections.push((title, body));
                }
            }
        }
    }

    if let Some(items) = patch.get("removeSections").and_then(JsonValue::as_array) {
        let removals = items
            .iter()
            .filter_map(JsonValue::as_str)
            .map(normalize_section_title)
            .filter(|value| !value.is_empty())
            .collect::<HashSet<_>>();
        sections.retain(|(title, _)| !removals.contains(&normalize_section_title(title)));
    }

    if let Some(items) = patch.get("addSections").and_then(JsonValue::as_array) {
        for item in items {
            if let Some((title, body)) = patch_section(item) {
                if normalize_section_title(&title) == normalize_section_title("学习补丁")
                    && !item
                        .as_str()
                        .is_some_and(|value| value.trim_start().starts_with('#'))
                {
                    add_without_heading.push(body);
                    continue;
                }
                let key = normalize_section_title(&title);
                if let Some(existing) = sections
                    .iter_mut()
                    .find(|(old_title, _)| normalize_section_title(old_title) == key)
                {
                    if !existing.1.contains(&body) {
                        existing.1 = format!("{}\n{}", existing.1.trim_end(), body);
                    }
                } else {
                    sections.push((title, body));
                }
            }
        }
    }
    if let Some(instructions) = patch.get("instructions").and_then(JsonValue::as_str) {
        if !instructions.trim().is_empty() {
            add_without_heading.push(instructions.trim().to_string());
        }
    }
    if !add_without_heading.is_empty() {
        if let Some(existing) = sections.iter_mut().find(|(title, _)| {
            normalize_section_title(title) == normalize_section_title("学习补丁")
        }) {
            for body in add_without_heading {
                if !existing.1.contains(&body) {
                    existing.1 = format!("{}\n{}", existing.1.trim_end(), body);
                }
            }
        } else {
            sections.push(("学习补丁".to_string(), add_without_heading.join("\n")));
        }
    }

    let mut output = Vec::new();
    if !preamble.is_empty() {
        output.push(preamble);
    }
    output.extend(
        sections
            .into_iter()
            .filter(|(_, body)| !body.trim().is_empty())
            .map(|(title, body)| format!("## {title}\n{}", body.trim())),
    );
    output.join("\n\n").trim().to_string()
}

pub(crate) fn skill_patch_has_content(patch: &JsonValue) -> bool {
    patch
        .get("instructions")
        .and_then(JsonValue::as_str)
        .is_some_and(|value| !value.trim().is_empty())
        || ["addSections", "replaceSections", "removeSections"]
            .iter()
            .any(|key| {
                patch
                    .get(key)
                    .and_then(JsonValue::as_array)
                    .is_some_and(|items| !items.is_empty())
            })
}

pub(crate) fn skill_compare_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

pub(crate) fn refine_learning_patch_for_apply(
    candidate: &JsonValue,
    base_instructions: &str,
    catalog: &str,
    complete: impl FnOnce(&str) -> Result<JsonValue, String>,
) -> Result<JsonValue, String> {
    let prompt = format!(
        "请对一个已经通过人工审核的 AppSec 学习候选做最终 Skill 精炼。必须和其它已有 Skill、历史高质量知识进行全局去重。若候选只是重复内容，decision=merge_existing 并给出 targetSkillId；若只是补充，合并到最匹配的 Skill；只有确实形成独立可复用能力时才 decision=create_new；没有新增价值时 decision=discard。删除一次性目标信息、重复步骤和无证据猜测，保留目标 Skill 未涉及的稳定章节。只输出 JSON，结构为 {{\"decision\":\"merge_existing|create_new|discard\",\"targetSkillId\":null,\"reasoning\":\"\",\"skillPatch\":{{\"addSections\":[],\"replaceSections\":[],\"removeSections\":[],\"keepSections\":[],\"reasoning\":\"\"}}}}。replaceSections 项优先使用 {{\"title\":\"章节名\",\"content\":\"内容\"}}。\n\n已审核候选：{}\n\n目标 Skill 当前内容：{}\n\n已有 Skill 与知识目录：{}",
        serde_json::to_string(candidate).unwrap_or_default(),
        base_instructions.chars().take(16_000).collect::<String>(),
        catalog
    );
    let refined = complete(&prompt)?;
    let patch = refined
        .get("skillPatch")
        .cloned()
        .unwrap_or_else(|| refined.clone());
    let mut output = patch;
    if let Some(object) = output.as_object_mut() {
        if let Some(value) = refined.get("decision") {
            object.insert("decision".into(), value.clone());
        }
        if let Some(value) = refined.get("targetSkillId") {
            object.insert("targetSkillId".into(), value.clone());
        }
        if let Some(value) = refined.get("reasoning") {
            object.insert("globalReasoning".into(), value.clone());
        }
    }
    if refined.get("decision").and_then(JsonValue::as_str) == Some("discard") {
        return Err("全局去重后没有发现新增可复用价值".into());
    }
    if skill_patch_has_content(&output) {
        Ok(output)
    } else {
        Err("最终精炼没有返回可应用的 Skill 补丁".into())
    }
}
