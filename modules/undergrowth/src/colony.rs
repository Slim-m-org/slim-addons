//! A colony is one player's fungus (or the wild rot): its name, colour, energy and plans.

use serde::{Deserialize, Serialize};

use crate::rng::hash;

/// Six players a season, so six colours stay tellable apart on a phone.
pub const MAX_PLAYERS: usize = 6;
/// Waypoints a drag can lay down for the hyphae to follow.
pub const MAX_WAYPOINTS: usize = 8;
pub const MAX_MUSHROOMS: u32 = 8;
pub const ENERGY_CAP: i32 = 240;
pub const WILD_ID: &str = "wild";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Colony {
    /// The first 16 characters of the player's opaque caller id, or `wild`.
    #[serde(rename = "p")]
    pub pid: String,
    #[serde(rename = "n")]
    pub name: String,
    /// The owner number its cells carry: 1 to 6 for players, 7 for the wild rot.
    #[serde(rename = "k")]
    pub slot: u8,
    #[serde(rename = "e")]
    pub energy: i32,
    /// The cell index of its heart, which nothing can take.
    #[serde(rename = "hr")]
    pub heart: u32,
    #[serde(rename = "wp", default)]
    pub waypoints: Vec<u32>,
    /// Ticks of growth spurt left.
    #[serde(rename = "b", default)]
    pub burst: u16,
    /// Points from mushrooms that have stood a while.
    #[serde(rename = "s", default)]
    pub score: i32,
    #[serde(rename = "cl", default)]
    pub cells: u32,
    #[serde(rename = "m", default)]
    pub mushrooms: u32,
    #[serde(rename = "pk", default)]
    pub peak: u32,
}

impl Colony {
    pub fn is_wild(&self) -> bool {
        self.slot == crate::world::WILD
    }

    /// What a season ranks on: territory, mushrooms standing, and what they earned.
    pub fn total(&self) -> i32 {
        self.cells as i32 + 25 * self.mushrooms as i32 + self.score
    }
}

/// The short id a colony stores for a caller: enough to tell players apart, nothing more.
pub fn pid_of(caller: &str) -> String {
    caller.chars().filter(char::is_ascii_alphanumeric).take(16).collect()
}

const ADJECTIVES: [&str; 20] = [
    "Amber", "Velvet", "Ghost", "Ink", "Rust", "Honey", "Frost", "Ember", "Moss", "Pale", "Ash", "Saffron",
    "Violet", "Copper", "Lantern", "Hollow", "Bitter", "Silver", "Shaggy", "Weeping",
];
const GENERA: [&str; 16] = [
    "Bonnet", "Cap", "Shank", "Rot", "Puffball", "Webcap", "Brittlegill", "Funnel", "Morel", "Chanterelle",
    "Truffle", "Tooth", "Bracket", "Stinkhorn", "Waxcap", "Milkcap",
];

/// A fungal name a player keeps until they choose one.
pub fn default_name(pid: &str, season: u32) -> String {
    let h = hash(format!("{pid}:{season}").as_bytes());
    format!("{} {}", ADJECTIVES[(h % 20) as usize], GENERA[((h >> 8) % 16) as usize])
}

/// A name a player typed, kept to letters, digits, spaces, hyphens and apostrophes.
pub fn clean_name(raw: &str) -> Option<String> {
    let kept: String = raw
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '\''))
        .take(16)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!kept.is_empty()).then_some(kept)
}

/// The colour of owner `slot`, as `#rrggbb`. Bright enough on dark soil in either theme.
pub fn colour(slot: u8) -> &'static str {
    match slot {
        1 => "#e0a040",
        2 => "#a57cf0",
        3 => "#2fc2b0",
        4 => "#ef6a86",
        5 => "#a3d14f",
        6 => "#5aaef0",
        _ => "#9a9c70",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pid_is_a_short_safe_prefix() {
        assert_eq!(pid_of("ab\"cd-ef0123456789abcdef0123"), "abcdef0123456789");
        assert_eq!(pid_of(""), "");
    }

    #[test]
    fn default_names_are_stable_and_vary_by_season() {
        assert_eq!(default_name("abc", 1), default_name("abc", 1));
        let names: std::collections::HashSet<_> = (1..20).map(|s| default_name("abc", s)).collect();
        assert!(names.len() > 5);
    }

    #[test]
    fn typed_names_are_cleaned_and_bounded() {
        assert_eq!(clean_name("  Moss   <b>Lord</b> "), Some("Moss bLordb".into()));
        assert_eq!(clean_name("!!!"), None);
        assert_eq!(clean_name(&"x".repeat(50)).unwrap().len(), 16);
        assert_eq!(clean_name("O'Rourke-2"), Some("O'Rourke-2".into()));
    }

    #[test]
    fn a_colony_totals_cells_mushrooms_and_score() {
        let c = Colony { cells: 100, mushrooms: 2, score: 7, ..Colony::default() };
        assert_eq!(c.total(), 157);
    }
}
