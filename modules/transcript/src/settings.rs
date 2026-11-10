//! The admin's settings, as the host sends them on every run.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub keep_days: i64,
    pub merge_gap_s: i64,
    pub timestamps: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { keep_days: 30, merge_gap_s: 30, timestamps: true }
    }
}

impl Settings {
    /// Reads the `settings` object of a run; anything missing or out of range falls back to the manifest's default or its bound.
    pub fn from_value(value: &Value) -> Settings {
        let d = Settings::default();
        let int = |key: &str, default: i64, lo: i64, hi: i64| value.get(key).and_then(Value::as_i64).unwrap_or(default).clamp(lo, hi);
        Settings {
            keep_days: int("keep_days", d.keep_days, 1, 90),
            merge_gap_s: int("merge_gap_s", d.merge_gap_s, 1, 300),
            timestamps: value.get("timestamps").and_then(Value::as_bool).unwrap_or(d.timestamps),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_settings_take_the_defaults_and_out_of_range_ones_are_clamped() {
        assert_eq!(Settings::from_value(&json!({})), Settings::default());
        let wild = Settings::from_value(&json!({"keep_days": 4000, "merge_gap_s": -3, "timestamps": false}));
        assert_eq!(wild, Settings { keep_days: 90, merge_gap_s: 1, timestamps: false });
    }
}
