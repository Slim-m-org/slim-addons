//! Answers the server's `interceptor/1` input: the reject phase blocks, the rewrite phase
//! masks, and each phase stays out of the way when the admin picked the other action.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::rules::{Action, Rules};

#[derive(Deserialize)]
struct Input {
    #[serde(rename = "$slim")]
    contract: String,
    phase: String,
    #[serde(default)]
    message: Message,
}

#[derive(Deserialize, Default)]
struct Message {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    channel_id: Option<String>,
}

pub fn answer(rules: &Rules, input: &str) -> Result<String, String> {
    let input: Input = serde_json::from_str(input).map_err(|e| format!("not interceptor input: {e}"))?;
    if input.contract != "interceptor/1" {
        return Err(format!("unsupported contract {}", input.contract));
    }
    let content = input.message.content.unwrap_or_default();
    let exempt = rules.is_exempt(input.message.channel_id.as_deref());
    let reply: Value = match (input.phase.as_str(), rules.action) {
        ("reject", Action::Block) if !exempt && rules.matches(&content) => {
            json!({"action": "reject", "reason": rules.reason})
        }
        ("reject", _) => json!({"action": "allow"}),
        ("rewrite", Action::Mask) if !exempt => match rules.mask(&content) {
            masked if masked != content => json!({"content": masked}),
            _ => json!({}),
        },
        ("rewrite", _) => json!({}),
        (other, _) => return Err(format!("this module has no {other} phase")),
    };
    Ok(reply.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(phase: &str, content: &str, channel: &str) -> String {
        json!({"$slim": "interceptor/1", "hook": "message.pre_send", "phase": phase,
               "message": {"content": content, "channel_id": channel}})
        .to_string()
    }

    fn reply(settings: Value, phase: &str, content: &str) -> Value {
        let rules = Rules::from_settings(&settings).unwrap();
        serde_json::from_str(&answer(&rules, &input(phase, content, "c1")).unwrap()).unwrap()
    }

    #[test]
    fn mask_mode_rewrites_and_never_rejects() {
        let s = json!({"words": "darn", "action": "mask"});
        assert_eq!(reply(s.clone(), "rewrite", "darn it"), json!({"content": "**** it"}));
        assert_eq!(reply(s, "reject", "darn it"), json!({"action": "allow"}));
    }

    #[test]
    fn block_mode_rejects_with_the_reason_and_never_rewrites() {
        let s = json!({"words": "darn", "action": "block", "block_reason": "Keep it clean."});
        assert_eq!(reply(s.clone(), "reject", "darn it"), json!({"action": "reject", "reason": "Keep it clean."}));
        assert_eq!(reply(s, "rewrite", "darn it"), json!({}));
    }

    #[test]
    fn a_clean_message_is_left_alone_in_both_phases() {
        let s = json!({"words": "darn", "action": "block"});
        assert_eq!(reply(s.clone(), "reject", "hello"), json!({"action": "allow"}));
        assert_eq!(reply(json!({"words": "darn"}), "rewrite", "hello"), json!({}));
    }

    #[test]
    fn an_exempt_channel_is_untouched() {
        let rules = Rules::from_settings(&json!({"words": "darn", "action": "block", "exempt_channels": "c1"})).unwrap();
        let out: Value = serde_json::from_str(&answer(&rules, &input("reject", "darn", "c1")).unwrap()).unwrap();
        assert_eq!(out, json!({"action": "allow"}));
    }

    #[test]
    fn other_input_is_an_error_not_a_pass() {
        let rules = Rules::from_settings(&json!({})).unwrap();
        assert!(answer(&rules, "hello").is_err());
        assert!(answer(&rules, r#"{"$slim":"interceptor/2","phase":"reject"}"#).is_err());
        assert!(answer(&rules, r#"{"$slim":"interceptor/1","phase":"annotate"}"#).is_err());
    }
}
