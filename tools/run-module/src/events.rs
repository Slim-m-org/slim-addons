//! The `@event` run: what slim sends a module when a subscribed event happens.
//! The event names, their `data` fields and the delivery `id` follow
//! `docs/modules/reactive-modules.md` in slim-m.

use serde_json::{json, Value};

/// (event, the `data` fields it carries, the field that is its occurrence time).
const EVENTS: [(&str, &[&str], Option<&str>); 12] = [
    ("message.created", &["message_id", "channel_id", "author_id", "content", "created_at_ms", "is_bot"], Some("created_at_ms")),
    ("message.edited", &["message_id", "channel_id", "author_id", "content", "edited_at_ms", "is_bot"], Some("edited_at_ms")),
    ("message.deleted", &["message_id", "channel_id"], None),
    ("reaction.changed", &["message_id", "channel_id", "emoji", "count", "user_id", "added"], None),
    ("member.joined", &["user_id"], None),
    ("member.left", &["user_id"], None),
    ("member.roles.changed", &["user_id", "role_id", "granted", "role_name"], None),
    ("voice.participant.joined", &["channel_id", "user_id"], None),
    ("voice.participant.left", &["channel_id", "user_id"], None),
    ("voice.screen_share.changed", &["channel_id", "user_id", "sharing"], None),
    ("voice.session.closed", &["session_id", "user_id", "channel_id", "joined_at_ms", "left_at_ms", "alone_ms"], Some("left_at_ms")),
    ("poll.voted", &["message_id", "channel_id", "options", "ranked", "winner"], None),
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
    Ok(json!({"$slim": "event/1", "event": name, "id": id, "at_ms": at_ms, "data": data}).to_string())
}

fn delivery_id(name: &str, data: &Value, at_ms: i64) -> String {
    let text = |f: &str| data[f].as_str().map(str::to_owned).unwrap_or_else(|| data[f].to_string());
    match name {
        "message.created" => format!("message:{}", text("message_id")),
        // slim ends this id with the edit's op seq, which the data does not carry; the edit time stands in.
        "message.edited" => format!("message-edit:{}:{at_ms}", text("message_id")),
        "message.deleted" => format!("message-deleted:{}", text("message_id")),
        "reaction.changed" => format!("reaction:{}:{}:{at_ms}:{}", text("message_id"), text("emoji"), text("user_id")),
        "member.joined" => format!("member-join:{}:{at_ms}", text("user_id")),
        "member.left" => format!("member-leave:{}:{at_ms}", text("user_id")),
        "member.roles.changed" => format!("member-roles:{}:{}:{at_ms}", text("user_id"), text("role_id")),
        "voice.participant.joined" => format!("voice-join:{}:{at_ms}", text("user_id")),
        "voice.participant.left" => format!("voice-leave:{}:{at_ms}", text("user_id")),
        "voice.screen_share.changed" => format!("voice-share:{}:{at_ms}", text("user_id")),
        "poll.voted" => format!("poll-vote:{}:{at_ms}", text("message_id")),
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

    /// slim-m's docs/modules/event-fixtures, the input slim sends for each add-on api 5 event.
    const FIXTURES: [&str; 6] = [
        r#"{"$slim":"event/1","event":"member.roles.changed","id":"member-roles:u1:r1:1760000200000","at_ms":1760000200000,"data":{"user_id":"u1","role_id":"r1","granted":true,"role_name":"Mods"}}"#,
        r#"{"$slim":"event/1","event":"poll.voted","id":"poll-vote:m1:1760000400000","at_ms":1760000400000,"data":{"message_id":"m1","channel_id":"c1","options":[{"position":0,"votes":2},{"position":1,"votes":1}],"ranked":false,"winner":null}}"#,
        r#"{"$slim":"event/1","event":"reaction.changed","id":"reaction:m1:thumbs-up:1760000500000:u2","at_ms":1760000500000,"data":{"message_id":"m1","channel_id":"c1","emoji":"thumbs-up","count":3,"user_id":"u2","added":true}}"#,
        r#"{"$slim":"event/1","event":"voice.participant.joined","id":"voice-join:u1:1760000000000","at_ms":1760000000000,"data":{"channel_id":"c1","user_id":"u1"}}"#,
        r#"{"$slim":"event/1","event":"voice.participant.left","id":"voice-leave:u1:1760000600000","at_ms":1760000600000,"data":{"channel_id":"c1","user_id":"u1"}}"#,
        r#"{"$slim":"event/1","event":"voice.screen_share.changed","id":"voice-share:u1:1760000100000","at_ms":1760000100000,"data":{"channel_id":"c1","user_id":"u1","sharing":true}}"#,
    ];

    #[test]
    fn the_api_5_events_match_what_slim_sends() {
        for fixture in FIXTURES {
            let want: Value = serde_json::from_str(fixture).unwrap();
            let name = want["event"].as_str().unwrap();
            let at = want["at_ms"].as_i64().unwrap();
            let got: Value = serde_json::from_str(&input(name, &want["data"].to_string(), at, None).unwrap()).unwrap();
            assert_eq!(got, want, "{name}");
        }
    }

    #[test]
    fn an_edit_is_timed_by_its_edit() {
        let data = r#"{"message_id":"m1","channel_id":"c1","author_id":"u1","content":"x","edited_at_ms":40,"is_bot":false}"#;
        let sent: Value = serde_json::from_str(&input("message.edited", data, 9, None).unwrap()).unwrap();
        assert_eq!((sent["id"].as_str(), sent["at_ms"].as_i64()), (Some("message-edit:m1:40"), Some(40)));
    }

    #[test]
    fn an_event_without_its_own_clock_takes_the_run_clock() {
        let sent: Value = serde_json::from_str(&input("member.joined", r#"{"user_id":"u"}"#, 77, None).unwrap()).unwrap();
        assert_eq!(sent["id"], "member-join:u:77");
    }
}
