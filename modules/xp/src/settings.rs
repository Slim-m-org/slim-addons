//! The admin's settings, as the host sends them on every run.

use serde_json::Value;

use crate::levels::Curve;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub xp_per_minute: i64,
    pub only_with_others: bool,
    pub xp_per_message: i64,
    pub message_cooldown_s: i64,
    pub curve: Curve,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            xp_per_minute: 5,
            only_with_others: true,
            xp_per_message: 0,
            message_cooldown_s: 60,
            curve: Curve::Standard,
        }
    }
}

impl Settings {
    /// Reads the `settings` object of a run; anything missing or out of range falls back to the manifest's default or its bound.
    pub fn from_value(value: &Value) -> Settings {
        let d = Settings::default();
        let int = |key: &str, default: i64, lo: i64, hi: i64| {
            value.get(key).and_then(Value::as_i64).unwrap_or(default).clamp(lo, hi)
        };
        Settings {
            xp_per_minute: int("xp_per_minute", d.xp_per_minute, 0, 100),
            only_with_others: value.get("only_with_others").and_then(Value::as_bool).unwrap_or(d.only_with_others),
            xp_per_message: int("xp_per_message", d.xp_per_message, 0, 50),
            message_cooldown_s: int("message_cooldown_s", d.message_cooldown_s, 10, 600),
            curve: value.get("curve").and_then(Value::as_str).map_or(d.curve, Curve::parse),
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
    fn sent_values_are_read_and_clamped_to_their_range() {
        let s = Settings::from_value(&json!({
            "xp_per_minute": 500, "only_with_others": false, "xp_per_message": -3,
            "message_cooldown_s": 1, "curve": "steep"
        }));
        assert_eq!(
            s,
            Settings { xp_per_minute: 100, only_with_others: false, xp_per_message: 0, message_cooldown_s: 10, curve: Curve::Steep }
        );
    }
}
