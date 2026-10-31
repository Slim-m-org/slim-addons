//! The `@event` run: what slim sends a module when a subscribed event happens.
//! The event names, their `data` fields and the delivery `id` follow
//! `docs/modules/reactive-modules.md` in slim-m.

use serde_json::{json, Value};

/// (event, the `data` fields it carries, the field that is its occurrence time).
const EVENTS: [(&str, &[&str], Option<&str>); 6] = [
    ("message.created", &["message_id", "channel_id", "author_id", "content", "created_at_ms", "is_bot"], Some("created_at_ms")),
    ("message.deleted", &["message_id", "channel_id"], None),
    ("reaction.changed", &["message_id", "channel_id", "emoji", "count"], None),
    ("member.joined", &["user_id"], None),
    ("member.left", &["user_id"], None),
    ("voice.session.closed", &["session_id", "user_id", "channel_id", "joined_at_ms", "left_at_ms", "alone_ms"], Some("left_at_ms")),
];

/// The `input` of an `@event` run, or why slim would never have sent it.
pub fn input(name: &str, data: &str, now_ms: i64, manifest: Option<&Value>) -> Result<String, String> {
    let Some((_, fields, at_field)) = EVENTS.iter().find(|e| e.0 == name) else {
        return Err(format!("unknown event {name}; slim sends {}", EVENTS.map(|e| e.0).join(", ")));
    };
    if let Some(manifest) = manifest {
        let subscribed = manifest["subscriptions"].as_array().is_some_and(|s| s.iter().any(|s| s["event"] == name));
        if !subscribed {
            return Err(format!("the manifest does not subscribe to {name}, so slim would never send it"));
        }
    }
    let data: Value = serde_json::from_str(data).map_err(|e| format!("--event data is not JSON: {e}"))?;
    if let Some(missing) = fields.iter().find(|f| data.get(**f).is_none()) {
        return Err(format!("{name} data has no {missing}"));
    }
    let at_ms = at_field.and_then(|f| data[f].as_i64()).unwrap_or(now_ms);
    let id = delivery_id(name, &data, at_ms);
    Ok(json!({"event": name, "id": id, "at_ms": at_ms, "data": data}).to_string())
}

fn delivery_id(name: &str, data: &Value, at_ms: i64) -> String {
    let text = |f: &str| data[f].as_str().map(str::to_owned).unwrap_or_else(|| data[f].to_string());
    match name {
        "message.created" => format!("message:{}", text("message_id")),
        "message.deleted" => format!("message-deleted:{}", text("message_id")),
        "reaction.changed" => format!("reaction:{}:{}:{at_ms}", text("message_id"), text("emoji")),
        "member.joined" => format!("member-join:{}:{at_ms}", text("user_id")),
        "member.left" => format!("member-leave:{}:{at_ms}", text("user_id")),
        _ => format!("voice-session:{}", text("session_id")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = r#"{"session_id":41,"user_id":"u","channel_id":"c","joined_at_ms":1,"left_at_ms":600,"alone_ms":0}"#;

    fn manifest() -> Value {
        json!({"subscriptions": [{"event": "voice.session.closed"}]})
    }

    #[test]
    fn a_session_event_carries_its_delivery_id_and_the_time_it_closed() {
        let sent: Value = serde_json::from_str(&input("voice.session.closed", SESSION, 9, Some(&manifest())).unwrap()).unwrap();
        assert_eq!((sent["id"].as_str(), sent["at_ms"].as_i64()), (Some("voice-session:41"), Some(600)));
        assert_eq!(sent["data"]["user_id"], "u");
    }

    #[test]
    fn an_event_slim_would_not_send_is_refused() {
        assert!(input("voice.session.closed", SESSION, 9, Some(&json!({}))).unwrap_err().contains("does not subscribe"));
        assert!(input("member.joined", "{}", 9, None).unwrap_err().contains("no user_id"));
        assert!(input("nope", "{}", 9, None).unwrap_err().contains("unknown event"));
    }

    #[test]
    fn an_event_without_its_own_clock_takes_the_run_clock() {
        let sent: Value = serde_json::from_str(&input("member.joined", r#"{"user_id":"u"}"#, 77, None).unwrap()).unwrap();
        assert_eq!(sent["id"], "member-join:u:77");
    }
}
