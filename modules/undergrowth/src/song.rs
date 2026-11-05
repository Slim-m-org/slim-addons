//! The forest's song and the little cues a player hears for their own moves. Slim plays a
//! scene's notes only for the person whose press produced it, so a cue never sounds on anyone
//! else's say-so.

use serde_json::{json, Value};

use crate::world::World;

/// A major pentatonic ladder from A3, which keeps any two notes that land together agreeable.
fn pitch(step: usize) -> f32 {
    const SCALE: [f32; 5] = [0.0, 2.0, 4.0, 7.0, 9.0];
    let octave = (step / 5) as f32;
    220.0 * 2f32.powf(octave + SCALE[step % 5] / 12.0)
}

fn note(f: f32, t: f32, d: f32) -> Value {
    json!({"f": (f * 10.0).round() / 10.0, "t": (t * 100.0).round() / 100.0, "d": d})
}

/// Up to 32 mushrooms, spread across the soil, sing as a playhead sweeps it left to right: depth
/// sets the pitch, the colony the octave. Returns the notes and how long the sweep lasts.
pub fn forest(world: &World) -> (Vec<Value>, f32) {
    const SECS: f32 = 6.0;
    // Slim plays at most 32 notes from one op.
    const MAX_NOTES: usize = 32;
    let mut cells: Vec<usize> = (0..world.cells.len()).filter(|i| world.cells[*i].mushroom()).collect();
    cells.sort_by_key(|i| (i % world.w, i / world.w));
    let n = cells.len();
    let mut notes: Vec<Value> = (0..n.min(MAX_NOTES))
        .map(|k| &cells[k * n / n.min(MAX_NOTES)])
        .map(|i| {
            let (r, c) = (i / world.w, i % world.w);
            let depth = (world.h - 1 - r) * 9 / world.h;
            let owner = usize::from(world.cells[*i].owner() % 2);
            note(pitch(depth + owner * 5), c as f32 / world.w as f32 * SECS, 0.6)
        })
        .collect();
    if notes.is_empty() {
        notes = vec![note(pitch(0), 0.0, 1.2), note(pitch(2), 0.4, 1.2), note(pitch(4), 0.8, 1.8)];
    }
    (notes, SECS)
}

/// The cue for one of the presser's own moves.
pub fn cue(kind: &str) -> Vec<Value> {
    match kind {
        "join" | "season" => vec![note(pitch(5), 0.0, 0.4), note(pitch(7), 0.15, 0.4), note(pitch(9), 0.3, 0.7)],
        "lure" => vec![note(pitch(7), 0.0, 0.2), note(pitch(9), 0.08, 0.3)],
        "fruit" => (0..5).map(|k| note(pitch(5 + k * 2), k as f32 * 0.09, 0.5)).collect(),
        "burst" => vec![note(pitch(3), 0.0, 0.8), note(pitch(5), 0.0, 0.8), note(pitch(7), 0.0, 0.8)],
        "spore" => vec![note(pitch(12), 0.0, 0.2), note(pitch(10), 0.1, 0.2), note(pitch(13), 0.2, 0.5)],
        "took" => vec![note(pitch(2), 0.0, 0.25)],
        "lost" => vec![note(pitch(1), 0.0, 0.3), note(pitch(0), 0.12, 0.4)],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    #[test]
    fn pitches_climb_and_stay_inside_what_slim_plays() {
        assert!((pitch(0) - 220.0).abs() < 0.01);
        assert!((pitch(5) - 440.0).abs() < 0.01);
        assert!((1..30).all(|k| pitch(k) > pitch(k - 1)));
        assert!(pitch(24) < 7800.0);
    }

    #[test]
    fn a_silent_soil_still_hums_and_a_full_one_caps_at_thirty_two() {
        let mut world = World::generate(48, 30, 2);
        assert_eq!(forest(&world).0.len(), 3);
        for i in 0..world.cells.len() {
            world.cells[i].set_mushroom(true);
        }
        let (notes, secs) = forest(&world);
        assert_eq!(notes.len(), 32);
        let last = notes.iter().map(|n| n["t"].as_f64().unwrap()).fold(0.0, f64::max);
        assert!(last > f64::from(secs) * 0.9, "the rightmost mushrooms sing too, not only the leftmost 32: {last}");
        assert!(notes.iter().all(|n| n["t"].as_f64().unwrap() < f64::from(secs)));
    }

    #[test]
    fn every_move_that_succeeds_has_a_cue() {
        for kind in ["join", "lure", "fruit", "burst", "spore", "took", "lost"] {
            let notes = cue(kind);
            assert!(!notes.is_empty() && notes.len() <= 32, "{kind}");
        }
        assert!(cue("nothing").is_empty());
    }
}
