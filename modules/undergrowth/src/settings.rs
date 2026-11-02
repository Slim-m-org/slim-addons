//! The admin's settings, with the manifest's defaults filled in by slim and clamped again here,
//! since a setting with no value and no default is simply absent.

use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub world: String,
    pub tick_ms: i64,
    pub season_ticks: u32,
    pub redraw_s: u32,
    pub leaf_power: u32,
    pub wild_rot: bool,
    pub almanac: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            world: "medium".into(),
            tick_ms: 400,
            season_ticks: 2400,
            redraw_s: 3,
            leaf_power: 3,
            wild_rot: true,
            almanac: true,
        }
    }
}

impl Settings {
    pub fn from_value(v: &Value) -> Settings {
        let d = Settings::default();
        let int = |k: &str, lo: i64, hi: i64, def: i64| v[k].as_i64().map_or(def, |n| n.clamp(lo, hi));
        Settings {
            world: v["world"].as_str().filter(|w| ["small", "medium", "large", "huge"].contains(w)).unwrap_or(&d.world).to_owned(),
            tick_ms: int("tick_ms", 150, 5000, d.tick_ms),
            season_ticks: int("season_ticks", 200, 50_000, i64::from(d.season_ticks)) as u32,
            redraw_s: int("redraw_s", 2, 60, i64::from(d.redraw_s)) as u32,
            leaf_power: int("leaf_power", 0, 8, i64::from(d.leaf_power)) as u32,
            wild_rot: v["wild_rot"].as_bool().unwrap_or(d.wild_rot),
            almanac: v["almanac"].as_bool().unwrap_or(d.almanac),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_values_fall_back_and_wild_ones_are_clamped() {
        assert_eq!(Settings::from_value(&Value::Null), Settings::default());
        let s = Settings::from_value(&json!({"world": "galaxy", "tick_ms": 1, "season_ticks": 9_999_999, "redraw_s": 0, "wild_rot": false}));
        assert_eq!(s.world, "medium");
        assert_eq!((s.tick_ms, s.season_ticks, s.redraw_s), (150, 50_000, 2));
        assert!(!s.wild_rot);
    }

    #[test]
    fn a_valid_choice_is_kept() {
        assert_eq!(Settings::from_value(&json!({"world": "huge"})).world, "huge");
    }
}
