//! The admin's settings, as the host sends them on every run.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Days a finished transcript is kept before the daily prune removes it.
    pub keep_days: i64,
    /// Post the transcript to the posting channel when a call ends.
    pub post_when_done: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { keep_days: 30, post_when_done: true }
    }
}

impl Settings {
    /// Anything missing or out of range falls back to the manifest's default or its bound.
    pub fn from_value(value: &Value) -> Settings {
        let d = Settings::default();
        Settings {
            keep_days: value.get("keep_days").and_then(Value::as_i64).unwrap_or(d.keep_days).clamp(1, 365),
            post_when_done: value.get("post_when_done").and_then(Value::as_bool).unwrap_or(d.post_when_done),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nothing_sent_gives_the_manifest_defaults() {
        assert_eq!(Settings::from_value(&json!({})), Settings::default());
        assert_eq!(Settings::from_value(&Value::Null), Settings::default());
    }

    #[test]
    fn values_are_read_and_kept_in_range() {
        let s = Settings::from_value(&json!({"keep_days": 9999, "post_when_done": false}));
        assert_eq!(s, Settings { keep_days: 365, post_when_done: false });
        assert_eq!(Settings::from_value(&json!({"keep_days": 0})).keep_days, 1);
    }
}
