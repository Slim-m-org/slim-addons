//! The two values an admin sets; a missing or out-of-range one falls back rather than failing a run.

use serde_json::Value;

const DEFAULT_REMIND_MINUTES: i64 = 15;

pub struct Settings {
    pub utc_offset_min: i64,
    /// Zero turns reminders off.
    pub remind_min: i64,
}

impl Settings {
    pub fn from_value(value: &Value) -> Settings {
        Settings {
            utc_offset_min: value["utc_offset_minutes"].as_i64().unwrap_or(0).clamp(-720, 840),
            remind_min: value["remind_minutes"].as_i64().unwrap_or(DEFAULT_REMIND_MINUTES).clamp(0, 240),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn missing_values_fall_back_and_wild_ones_are_clamped() {
        let none = Settings::from_value(&Value::Null);
        assert_eq!((none.utc_offset_min, none.remind_min), (0, 15));
        let wild = Settings::from_value(&json!({"utc_offset_minutes": 99999, "remind_minutes": -4}));
        assert_eq!((wild.utc_offset_min, wild.remind_min), (840, 0));
    }
}
