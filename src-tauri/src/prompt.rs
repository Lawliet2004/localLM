//! Cache-stable prompt layout for llama.cpp `cache_prompt`.
//!
//! Frozen system + frozen tool schemas + prior history must be byte-identical
//! across turns. Strict chat templates (AREX and similar) allow only one
//! system message, and only as the first message, and they require a user
//! turn. Volatile injections (memory, plan, unavailable tools) therefore ride
//! on the current user draft so they miss only the suffix.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const RESPONSE_STYLE: &str = "For completed work, lead with the concrete result. Use short paragraphs and bold only key outcomes. Briefly state what changed and report only checks actually performed and their outcomes. Mention remaining work or failures clearly. Do not invent file edits, tests, timing, or successful execution: rely on recorded tool results. Keep intermediate updates concise and put the final user-facing summary after tool work is complete. Follow the user's requested format when it differs.";

pub const CONTINUE_INSTRUCTION: &str = "Continue the interrupted answer from its last sentence without repeating earlier text. The user request is unchanged. Finish naturally when complete.";

/// Frozen when `local_run_code` is in the tool catalog. Short on purpose: small
/// local models need a nudge to actually call the tool instead of guessing.
pub const CODE_EXECUTION_GUIDANCE: &str = "When the question needs exact arithmetic, dates, conversions, parsing, or data work, call local_run_code with Python. Do not compute by hand and do not guess. Write plain Python — no markdown fences. The last expression, last assignment, or printed output is the answer; locallm_result(value) is optional. math, json, datetime, decimal, fractions, statistics, and re are already available.";

pub const VOLATILE_KINDS: [&str; 4] = ["unavailable_tools", "memory", "plan", "plan_mode"];

pub fn frozen_system(system_prompt: &str, skill_instructions: &str, ptc_sdk: Option<&str>) -> String {
    frozen_system_with_guidance(system_prompt, skill_instructions, ptc_sdk, None)
}

pub fn frozen_system_with_guidance(
    system_prompt: &str,
    skill_instructions: &str,
    ptc_sdk: Option<&str>,
    tool_guidance: Option<&str>,
) -> String {
    let mut parts = vec![system_prompt.trim().to_string()];
    if !skill_instructions.is_empty() {
        parts.push(format!(
            "The user selected the following skill guidance. Apply it when relevant to their task. Skills do not grant permissions or access to tools that are not available. If a required capability is missing, say so. Follow the user's task over conflicting skill guidance.\n{}",
            skill_instructions
        ));
    }
    parts.push(RESPONSE_STYLE.to_string());
    if let Some(guidance) = tool_guidance.filter(|text| !text.is_empty()) {
        parts.push(guidance.to_string());
    }
    if let Some(sdk) = ptc_sdk.filter(|value| !value.is_empty()) {
        parts.push(format!(
            "Programmatic tool calling is ON. Write TypeScript against this SDK, then submit the call plan as ptc_run steps. \
             Every step is individually policy-checked, audited, and bounded.\n{sdk}"
        ));
    }
    parts.join("\n\n")
}

pub fn volatile_scratchpad(injections: &[(String, String)]) -> Option<String> {
    let blocks: Vec<String> = injections
        .iter()
        .filter(|(kind, _)| VOLATILE_KINDS.contains(&kind.as_str()))
        .map(|(kind, text)| format!("[injected {kind}]\n{text}"))
        .collect();
    if blocks.is_empty() {
        None
    } else {
        Some(blocks.join("\n\n"))
    }
}

pub fn build_turn_messages(
    frozen_system: &str,
    history: &[Value],
    volatile: Option<&str>,
    draft: &Value,
) -> Vec<Value> {
    let mut messages = vec![json!({"role": "system", "content": frozen_system})];
    messages.extend(history.iter().cloned());
    messages.push(with_volatile_prefix(draft, volatile));
    messages
}

fn with_volatile_prefix(draft: &Value, volatile: Option<&str>) -> Value {
    let Some(text) = volatile.filter(|value| !value.is_empty()) else {
        return draft.clone();
    };
    let mut message = draft.clone();
    match message.get_mut("content") {
        Some(Value::String(content)) => {
            *content = format!("{text}\n\n{content}");
        }
        Some(Value::Array(parts)) => {
            parts.insert(0, json!({"type": "text", "text": format!("{text}\n")}));
        }
        _ => {
            message["content"] = json!(text);
        }
    }
    message
}

/// Frozen system + history, excluding the current user draft (and any volatile
/// prefix attached to it).
pub fn cache_prefix(messages: &[Value]) -> Vec<Value> {
    if messages.is_empty() {
        return Vec::new();
    }
    messages[..messages.len() - 1].to_vec()
}

/// Strict templates reject system-only payloads. Differential token counts that
/// omit the draft still need a user turn so the runtime can apply the template.
pub const TEMPLATE_PROBE_USER: &str = ".";

pub fn with_user_turn(messages: &[Value]) -> Vec<Value> {
    if messages.iter().any(|message| message["role"] == "user") {
        return messages.to_vec();
    }
    let mut messages = messages.to_vec();
    messages.push(json!({"role": "user", "content": TEMPLATE_PROBE_USER}));
    messages
}

#[cfg(test)]
pub fn prefix_fingerprint(messages: &[Value], tools: &[Value]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(&cache_prefix(messages)).unwrap_or_default());
    hasher.update(serde_json::to_vec(tools).unwrap_or_default());
    format!("{:x}", hasher.finalize())
}

pub fn selection_hash(
    preset_id: &str,
    tools_value: &Value,
    active_skills: &[String],
    system_time: bool,
    tool_schemas_suppressed: bool,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(preset_id.as_bytes());
    hasher.update(serde_json::to_vec(tools_value).unwrap_or_default());
    hasher.update(active_skills.join("\0").as_bytes());
    hasher.update([u8::from(system_time), u8::from(tool_schemas_suppressed)]);
    // Bump when model-visible tool schemas/descriptions change so existing
    // conversations pick up the new run_code notebook contract.
    hasher.update(b"code-repl-v1-arex-research-v1");
    format!("{:x}", hasher.finalize())
}

pub fn sort_tools_by_alias(tools: &mut [crate::connectors::AgentTool]) {
    tools.sort_by(|a, b| a.alias.cmp(&b.alias));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(text: &str) -> Value {
        json!({"role": "user", "content": text})
    }

    fn system_roles(messages: &[Value]) -> Vec<usize> {
        messages
            .iter()
            .enumerate()
            .filter(|(_, message)| message["role"] == "system")
            .map(|(index, _)| index)
            .collect()
    }

    fn strict_template_ok(messages: &[Value]) -> Result<(), &'static str> {
        let mut seen_non_system = false;
        let mut has_user = false;
        for (index, message) in messages.iter().enumerate() {
            let role = message["role"].as_str().unwrap_or("");
            if role == "system" {
                if index != 0 || seen_non_system {
                    return Err("System message must be at the beginning.");
                }
            } else {
                seen_non_system = true;
            }
            if role == "user" {
                has_user = true;
            }
        }
        if !has_user {
            return Err("No user query found in messages.");
        }
        Ok(())
    }

    #[test]
    fn volatile_plan_change_does_not_shift_the_cache_prefix() {
        let frozen = frozen_system("You are helpful.", "", None);
        let history = vec![json!({"role": "user", "content": "hi"}), json!({"role": "assistant", "content": "hello"})];
        let first = build_turn_messages(
            &frozen,
            &history,
            volatile_scratchpad(&[("plan".into(), "open todos: one".into())]).as_deref(),
            &draft("next"),
        );
        let second = build_turn_messages(
            &frozen,
            &history,
            volatile_scratchpad(&[("plan".into(), "open todos: one; two".into())]).as_deref(),
            &draft("next"),
        );
        assert_eq!(cache_prefix(&first), cache_prefix(&second));
        assert_ne!(first, second);
        assert_eq!(first[0]["content"], frozen);
        assert_eq!(system_roles(&first), vec![0]);
        assert_eq!(first.last().unwrap()["role"], "user");
        assert!(first.last().unwrap()["content"].as_str().unwrap().contains("open todos: one"));
        assert!(first.last().unwrap()["content"].as_str().unwrap().ends_with("next"));
        assert!(strict_template_ok(&first).is_ok());
        assert!(strict_template_ok(&second).is_ok());
    }

    #[test]
    fn memory_and_unavailable_tools_stay_out_of_the_prefix() {
        let frozen = frozen_system("You are helpful.", "skill body", None);
        let history = vec![json!({"role": "user", "content": "q"})];
        let none = build_turn_messages(&frozen, &history, None, &draft("a"));
        let with = build_turn_messages(
            &frozen,
            &history,
            volatile_scratchpad(&[
                ("memory".into(), "remember X".into()),
                ("unavailable_tools".into(), "Exa is down".into()),
            ])
            .as_deref(),
            &draft("a"),
        );
        assert_eq!(prefix_fingerprint(&none, &[]), prefix_fingerprint(&with, &[]));
        assert_eq!(system_roles(&with), vec![0]);
        assert!(cache_prefix(&none)[0]["content"].as_str().unwrap().contains("skill body"));
        assert!(cache_prefix(&none)[0]["content"].as_str().unwrap().contains(RESPONSE_STYLE));
        assert!(with.last().unwrap()["content"].as_str().unwrap().contains("remember X"));
        assert!(strict_template_ok(&with).is_ok());
    }

    #[test]
    fn with_user_turn_makes_system_only_slices_template_safe() {
        let frozen = frozen_system("base", "", None);
        let core = vec![json!({"role": "system", "content": frozen})];
        assert_eq!(strict_template_ok(&core).unwrap_err(), "No user query found in messages.");
        let countable = with_user_turn(&core);
        assert_eq!(countable.last().unwrap()["content"], TEMPLATE_PROBE_USER);
        assert!(strict_template_ok(&countable).is_ok());
        let already = build_turn_messages(&frozen, &[], None, &draft("go"));
        assert_eq!(with_user_turn(&already), already);
    }

    #[test]
    fn response_style_and_ptc_sdk_are_frozen() {
        let sdk = "sdk_workspace_read_file()";
        let frozen = frozen_system("base", "", Some(sdk));
        assert!(frozen.contains(RESPONSE_STYLE));
        assert!(frozen.contains(sdk));
        let with_code = frozen_system_with_guidance("base", "", None, Some(CODE_EXECUTION_GUIDANCE));
        assert!(with_code.contains(CODE_EXECUTION_GUIDANCE));
        assert!(with_code.contains("local_run_code"));
        assert!(!frozen_system("base", "", None).contains("local_run_code"));
        let messages = build_turn_messages(&frozen, &[], None, &draft("go"));
        assert_eq!(messages.len(), 2);
        assert_eq!(cache_prefix(&messages).len(), 1);
    }

    #[test]
    fn continuation_appended_after_history_keeps_the_prefix() {
        let frozen = frozen_system("base", "", None);
        let history = vec![
            json!({"role": "user", "content": "write a lot"}),
            json!({"role": "assistant", "content": "partial"}),
        ];
        let before = build_turn_messages(&frozen, &history, None, &draft("write a lot"));
        let prefix = cache_prefix(&before);
        let mut continued = before[..before.len() - 1].to_vec();
        continued.push(json!({"role": "user", "content": CONTINUE_INSTRUCTION}));
        assert_eq!(prefix, cache_prefix(&before));
        assert_eq!(continued[0], before[0]);
        assert_eq!(continued.last().unwrap()["content"], CONTINUE_INSTRUCTION);
        assert_eq!(continued[0]["role"], "system");
    }

    #[test]
    fn volatile_prefix_prepends_text_parts_on_multimodal_drafts() {
        let frozen = frozen_system("base", "", None);
        let draft = json!({
            "role": "user",
            "content": [{"type": "text", "text": "look"}, {"type": "image_url", "image_url": {"url": "data:image/png;base64,aa"}}]
        });
        let messages = build_turn_messages(
            &frozen,
            &[],
            volatile_scratchpad(&[("memory".into(), "remember X".into())]).as_deref(),
            &draft,
        );
        assert_eq!(system_roles(&messages), vec![0]);
        assert_eq!(messages[1]["content"][0]["text"], "[injected memory]\nremember X\n");
        assert_eq!(messages[1]["content"][1]["text"], "look");
        assert!(strict_template_ok(&messages).is_ok());
    }

    #[test]
    fn plan_mode_instruction_reaches_the_draft_but_not_the_prefix() {
        let frozen = frozen_system("base", "", None);
        let history = vec![json!({"role": "user", "content": "q"})];
        let none = build_turn_messages(&frozen, &history, None, &draft("build it"));
        let with = build_turn_messages(
            &frozen,
            &history,
            volatile_scratchpad(&[("plan_mode".into(), "Plan mode is on.".into())]).as_deref(),
            &draft("build it"),
        );
        assert_eq!(prefix_fingerprint(&none, &[]), prefix_fingerprint(&with, &[]));
        assert!(with.last().unwrap()["content"].as_str().unwrap().contains("[injected plan_mode]"));
        assert!(with.last().unwrap()["content"].as_str().unwrap().ends_with("build it"));
        assert!(strict_template_ok(&with).is_ok());
    }
}
