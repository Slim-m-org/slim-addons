//! The initiative order: who is in the fight, whose turn it is, and which round.
//! Pure state; the scene lives in `scene.rs`.

use serde::{Deserialize, Serialize};

pub const MAX_COMBATANTS: usize = 24;
pub const MAX_NAME_CHARS: usize = 20;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Combatant {
    pub name: String,
    pub init: i32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Tracker {
    pub list: Vec<Combatant>,
    pub turn: usize,
    pub round: u32,
}

impl Default for Tracker {
    fn default() -> Self {
        Tracker { list: Vec::new(), turn: 0, round: 1 }
    }
}

impl Tracker {
    /// Reads the state a scene handed back; anything unreadable starts a fresh fight.
    pub fn from_state(state: &str) -> Tracker {
        let mut tracker: Tracker = serde_json::from_str(state).unwrap_or_default();
        tracker.list.truncate(MAX_COMBATANTS);
        if tracker.turn >= tracker.list.len() {
            tracker.turn = 0;
        }
        tracker.round = tracker.round.max(1);
        tracker
    }

    pub fn state(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn current(&self) -> Option<&Combatant> {
        self.list.get(self.turn)
    }

    pub fn add(&mut self, name: &str, init: i32) -> Result<(), String> {
        let name: String = name.chars().filter(|c| !c.is_control()).take(MAX_NAME_CHARS).collect();
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err("give the combatant a name".to_string());
        }
        if self.list.len() >= MAX_COMBATANTS {
            return Err(format!("the tracker holds at most {MAX_COMBATANTS} combatants"));
        }
        self.list.push(Combatant { name, init });
        Ok(())
    }

    /// Highest initiative first; ties keep the order they were added in.
    /// Before anyone has acted the top of the order goes first, afterwards the turn stays with whoever had it.
    pub fn sort(&mut self) {
        let fresh = self.round == 1 && self.turn == 0;
        let mut order: Vec<usize> = (0..self.list.len()).collect();
        order.sort_by_key(|&i| (std::cmp::Reverse(self.list[i].init), i));
        let turn = order.iter().position(|&i| i == self.turn).unwrap_or(0);
        self.list = order.iter().map(|&i| self.list[i].clone()).collect();
        self.turn = if fresh { 0 } else { turn };
    }

    /// Passes the turn on; the round counts up each time it wraps back to the top.
    pub fn next(&mut self) {
        if self.list.is_empty() {
            return;
        }
        self.turn += 1;
        if self.turn >= self.list.len() {
            self.turn = 0;
            self.round += 1;
        }
    }

    pub fn set_turn(&mut self, index: usize) {
        if index < self.list.len() {
            self.turn = index;
        }
    }

    pub fn remove(&mut self, index: usize) -> Option<Combatant> {
        if index >= self.list.len() {
            return None;
        }
        let gone = self.list.remove(index);
        if index < self.turn {
            self.turn -= 1;
        } else if self.turn >= self.list.len() {
            self.turn = 0;
            if !self.list.is_empty() {
                self.round += 1;
            }
        }
        Some(gone)
    }

    pub fn clear(&mut self) {
        *self = Tracker::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fight(entries: &[(&str, i32)]) -> Tracker {
        let mut tracker = Tracker::default();
        for (name, init) in entries {
            tracker.add(name, *init).unwrap();
        }
        tracker
    }

    fn names(tracker: &Tracker) -> Vec<&str> {
        tracker.list.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn sorting_puts_the_highest_first_and_keeps_ties_in_added_order() {
        let mut t = fight(&[("Ann", 12), ("Bo", 18), ("Cy", 12), ("Di", -1)]);
        t.sort();
        assert_eq!(names(&t), ["Bo", "Ann", "Cy", "Di"]);
    }

    #[test]
    fn sorting_does_not_take_the_turn_from_whoever_had_it() {
        let mut t = fight(&[("Ann", 5), ("Bo", 18), ("Cy", 9)]);
        t.set_turn(2);
        t.sort();
        assert_eq!(t.current().unwrap().name, "Cy");
        t.next();
        t.next();
        t.sort();
        assert_eq!((t.current().unwrap().name.as_str(), t.round), ("Bo", 2));
    }

    #[test]
    fn sorting_before_anyone_has_acted_starts_with_the_highest() {
        let mut t = fight(&[("Ann", 5), ("Bo", 18)]);
        t.sort();
        assert_eq!(t.current().unwrap().name, "Bo");
    }

    #[test]
    fn next_walks_the_order_and_counts_a_round_on_each_wrap() {
        let mut t = fight(&[("Ann", 3), ("Bo", 2)]);
        assert_eq!((t.turn, t.round), (0, 1));
        t.next();
        assert_eq!((t.turn, t.round), (1, 1));
        t.next();
        assert_eq!((t.turn, t.round), (0, 2));
    }

    #[test]
    fn next_on_an_empty_tracker_does_nothing() {
        let mut t = Tracker::default();
        t.next();
        assert_eq!(t, Tracker::default());
    }

    #[test]
    fn removing_someone_before_the_turn_keeps_the_turn_on_the_same_person() {
        let mut t = fight(&[("Ann", 3), ("Bo", 2), ("Cy", 1)]);
        t.set_turn(2);
        t.remove(0);
        assert_eq!(t.current().unwrap().name, "Cy");
    }

    #[test]
    fn removing_the_current_combatant_passes_the_turn_to_the_next() {
        let mut t = fight(&[("Ann", 3), ("Bo", 2), ("Cy", 1)]);
        t.set_turn(1);
        assert_eq!(t.remove(1).unwrap().name, "Bo");
        assert_eq!(t.current().unwrap().name, "Cy");
        assert_eq!(t.round, 1);
    }

    #[test]
    fn removing_the_last_combatant_on_their_turn_wraps_into_a_new_round() {
        let mut t = fight(&[("Ann", 3), ("Bo", 2)]);
        t.set_turn(1);
        t.remove(1);
        assert_eq!((t.turn, t.round), (0, 2));
        t.remove(0);
        assert_eq!((t.list.len(), t.round), (0, 2));
        assert!(t.remove(5).is_none());
    }

    #[test]
    fn clear_starts_a_fresh_fight() {
        let mut t = fight(&[("Ann", 3)]);
        t.next();
        t.clear();
        assert_eq!(t, Tracker::default());
    }

    #[test]
    fn names_are_cleaned_and_the_roster_is_capped() {
        let mut t = Tracker::default();
        assert!(t.add("   ", 1).is_err());
        t.add("Very long goblin chieftain name", 1).unwrap();
        assert_eq!(t.list[0].name.chars().count(), MAX_NAME_CHARS);
        for i in 1..MAX_COMBATANTS {
            t.add(&format!("g{i}"), 0).unwrap();
        }
        assert!(t.add("one more", 0).unwrap_err().contains("at most"));
    }

    #[test]
    fn state_round_trips_and_garbage_starts_fresh() {
        let mut t = fight(&[("Ann", 3), ("Bo", 2)]);
        t.next();
        assert_eq!(Tracker::from_state(&t.state()), t);
        assert_eq!(Tracker::from_state("not json"), Tracker::default());
        assert_eq!(Tracker::from_state(r#"{"list":[],"turn":9,"round":0}"#).round, 1);
    }
}
