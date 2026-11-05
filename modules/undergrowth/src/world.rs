//! The soil: a grid of packed cells, the terrain a season is dealt, and the text form it rides in.
//!
//! A cell is 16 bits so a whole world fits in the scene's `state` string: a 128x80 world is
//! 20 KiB of cells, 27 KiB once base64'd, under the 64 KiB a stored frame may be.

use crate::b64;
use crate::rng::{mix, Rng};

pub const AIR: u8 = 0;
pub const LOAM: u8 = 1;
pub const CLAY: u8 = 2;
pub const ROCK: u8 = 3;
pub const WATER: u8 = 4;
pub const VEIN: u8 = 5;

/// Rows of open air above the soil, where leaves land and nothing grows.
pub const SKY: usize = 2;
/// Owner 7 is the wild rot, a colony nobody plays.
pub const WILD: u8 = 7;
pub const MAX_NUTRIENT: u8 = 15;

/// One packed cell: terrain (3 bits), nutrient (4), owner (3), strength (2), and three flags.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Cell(pub u16);

impl Cell {
    pub fn new(terrain: u8, nutrient: u8) -> Cell {
        let mut c = Cell(0);
        c.set_terrain(terrain);
        c.set_nutrient(nutrient);
        c
    }
    pub fn terrain(self) -> u8 {
        (self.0 & 0x7) as u8
    }
    pub fn nutrient(self) -> u8 {
        ((self.0 >> 3) & 0xf) as u8
    }
    pub fn owner(self) -> u8 {
        ((self.0 >> 7) & 0x7) as u8
    }
    pub fn strength(self) -> u8 {
        ((self.0 >> 10) & 0x3) as u8
    }
    pub fn mushroom(self) -> bool {
        self.0 & (1 << 12) != 0
    }
    pub fn dead(self) -> bool {
        self.0 & (1 << 13) != 0
    }
    /// A heart or an outpost: what keeps the hyphae around it alive.
    pub fn anchor(self) -> bool {
        self.0 & (1 << 14) != 0
    }
    fn set_bits(&mut self, shift: u16, width: u16, value: u16) {
        let mask = ((1u16 << width) - 1) << shift;
        self.0 = (self.0 & !mask) | ((value << shift) & mask);
    }
    pub fn set_terrain(&mut self, t: u8) {
        self.set_bits(0, 3, u16::from(t));
    }
    pub fn set_nutrient(&mut self, n: u8) {
        self.set_bits(3, 4, u16::from(n.min(MAX_NUTRIENT)));
    }
    pub fn set_owner(&mut self, o: u8) {
        self.set_bits(7, 3, u16::from(o));
    }
    pub fn set_strength(&mut self, s: u8) {
        self.set_bits(10, 2, u16::from(s.min(3)));
    }
    pub fn set_mushroom(&mut self, on: bool) {
        self.set_bits(12, 1, u16::from(on));
    }
    pub fn set_dead(&mut self, on: bool) {
        self.set_bits(13, 1, u16::from(on));
    }
    pub fn set_anchor(&mut self, on: bool) {
        self.set_bits(14, 1, u16::from(on));
    }
    pub fn passable(self) -> bool {
        !matches!(self.terrain(), AIR | ROCK)
    }
    /// Empties a cell of any colony, keeping its soil.
    pub fn clear_colony(&mut self) {
        self.set_owner(0);
        self.set_strength(0);
        self.set_mushroom(false);
        self.set_anchor(false);
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct World {
    pub w: usize,
    pub h: usize,
    pub cells: Vec<Cell>,
}

/// The world sizes an admin can pick, as (columns, rows).
pub fn size_for(name: &str) -> (usize, usize) {
    match name {
        "small" => (48, 30),
        "large" => (96, 60),
        "huge" => (128, 80),
        _ => (64, 40),
    }
}

impl World {
    pub fn idx(&self, r: usize, c: usize) -> usize {
        r * self.w + c
    }

    #[cfg(test)]
    pub fn at(&self, r: usize, c: usize) -> Cell {
        self.cells[self.idx(r, c)]
    }

    /// The up-to-four neighbours of `i`, as indices.
    pub fn neighbours(&self, i: usize) -> impl Iterator<Item = usize> {
        let (w, h) = (self.w, self.h);
        let (r, c) = (i / w, i % w);
        let up = (r > 0).then(|| i - w);
        let down = (r + 1 < h).then(|| i + w);
        let left = (c > 0).then(|| i - 1);
        let right = (c + 1 < w).then(|| i + 1);
        [up, down, left, right].into_iter().flatten()
    }

    /// Deals a fresh world from `seed`: blobs of rock that thicken with depth, pockets of water,
    /// seams of clay, and gold veins of nutrient that regrow.
    pub fn generate(w: usize, h: usize, seed: u64) -> World {
        let mut cells = vec![Cell::default(); w * h];
        let soil_rows = (h - SKY) as f32;
        for r in 0..h {
            for c in 0..w {
                let i = r * w + c;
                if r < SKY {
                    cells[i] = Cell::new(AIR, 0);
                    continue;
                }
                let depth = (r - SKY) as f32 / soil_rows;
                let (x, y) = (c as f32, r as f32);
                let rock = fbm(seed, x / 9.0, y / 7.0);
                let wet = fbm(seed ^ 0x5157, x / 6.0, y / 5.0);
                let clay = fbm(seed ^ 0xc1a7, x / 11.0, y / 4.0);
                let rich = fbm(seed ^ 0xfeed, x / 5.0, y / 5.0);
                let top = if r < SKY + 3 { 4 } else { 0 };
                cells[i] = if r > SKY + 1 && rock > 0.76 - 0.12 * depth {
                    Cell::new(ROCK, 0)
                } else if depth > 0.2 && wet > 0.74 {
                    Cell::new(WATER, 5)
                } else if clay > 0.62 {
                    Cell::new(CLAY, 2 + (rich * 3.0) as u8)
                } else {
                    Cell::new(LOAM, 1 + (rich * 5.0) as u8 + top)
                };
            }
        }
        let mut world = World { w, h, cells };
        world.lay_veins(seed);
        world
    }

    fn lay_veins(&mut self, seed: u64) {
        let mut rng = Rng::new(mix(&[seed, 0x7e1d]));
        let walks = (self.w * self.h / 360).max(3);
        for _ in 0..walks {
            let mut r = SKY as i32 + 4 + rng.below((self.h - SKY - 5) as u32) as i32;
            let mut c = rng.below(self.w as u32) as i32;
            for _ in 0..(5 + rng.below(9)) {
                if r <= SKY as i32 || r >= self.h as i32 || c < 0 || c >= self.w as i32 {
                    break;
                }
                let i = self.idx(r as usize, c as usize);
                if self.cells[i].terrain() != ROCK {
                    self.cells[i] = Cell::new(VEIN, 12);
                }
                match rng.below(4) {
                    0 => r += 1,
                    1 => r -= 1,
                    2 => c += 1,
                    _ => c -= 1,
                }
            }
        }
    }

    pub fn encode(&self) -> String {
        let mut bytes = Vec::with_capacity(self.cells.len() * 2);
        for cell in &self.cells {
            bytes.extend_from_slice(&cell.0.to_le_bytes());
        }
        b64::encode(&bytes)
    }

    /// The world a state string carries, or `None` if it is not one this module could have written.
    pub fn decode(w: usize, h: usize, text: &str) -> Option<World> {
        if !valid_size(w, h) {
            return None;
        }
        let bytes = b64::decode(text)?;
        if bytes.len() != w * h * 2 {
            return None;
        }
        let cells = bytes.chunks_exact(2).map(|b| Cell(u16::from_le_bytes([b[0], b[1]]) & 0x7fff)).collect();
        Some(World { w, h, cells })
    }
}

pub fn valid_size(w: usize, h: usize) -> bool {
    (16..=128).contains(&w) && (12..=80).contains(&h)
}

/// Smooth value noise in `0..1`, three octaves.
fn fbm(seed: u64, x: f32, y: f32) -> f32 {
    let mut total = 0.0;
    let mut amp = 0.5;
    let mut freq = 1.0;
    for octave in 0..3u64 {
        total += amp * value_noise(seed.wrapping_add(octave * 0x9e37), x * freq, y * freq);
        amp *= 0.5;
        freq *= 2.0;
    }
    total / 0.875
}

fn value_noise(seed: u64, x: f32, y: f32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (smooth(x - x0), smooth(y - y0));
    let corner = |dx: f32, dy: f32| {
        let v = mix(&[seed, (x0 + dx) as i64 as u64, (y0 + dy) as i64 as u64]);
        (v >> 40) as f32 / (1u64 << 24) as f32
    };
    let top = lerp(corner(0.0, 0.0), corner(1.0, 0.0), fx);
    let bottom = lerp(corner(0.0, 1.0), corner(1.0, 1.0), fx);
    lerp(top, bottom, fy)
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cell_keeps_each_field_apart() {
        let mut c = Cell::new(VEIN, 12);
        c.set_owner(5);
        c.set_strength(2);
        c.set_mushroom(true);
        c.set_anchor(true);
        assert_eq!((c.terrain(), c.nutrient(), c.owner(), c.strength()), (VEIN, 12, 5, 2));
        assert!(c.mushroom() && c.anchor() && !c.dead());
        c.set_nutrient(99);
        assert_eq!(c.nutrient(), MAX_NUTRIENT);
        c.clear_colony();
        assert_eq!((c.owner(), c.strength(), c.mushroom(), c.anchor()), (0, 0, false, false));
        assert_eq!(c.terrain(), VEIN);
    }

    #[test]
    fn the_same_seed_deals_the_same_world_and_another_seed_does_not() {
        let a = World::generate(64, 40, 42);
        assert_eq!(a, World::generate(64, 40, 42));
        assert_ne!(a, World::generate(64, 40, 43));
    }

    #[test]
    fn a_world_has_sky_soil_rock_water_and_veins() {
        let w = World::generate(64, 40, 9);
        assert!((0..64).all(|c| w.at(0, c).terrain() == AIR && w.at(1, c).terrain() == AIR));
        let count = |t| w.cells.iter().filter(|c| c.terrain() == t).count();
        let soil = 64 * 38;
        assert!(count(LOAM) > soil / 3, "loam {}", count(LOAM));
        assert!(count(ROCK) > 20 && count(ROCK) < soil / 3, "rock {}", count(ROCK));
        assert!(count(VEIN) > 5, "veins {}", count(VEIN));
        assert!(count(WATER) > 0 && count(CLAY) > 0);
    }

    #[test]
    fn a_world_round_trips_through_its_text() {
        let w = World::generate(48, 30, 1);
        let text = w.encode();
        assert_eq!(World::decode(48, 30, &text), Some(w));
        assert_eq!(World::decode(48, 31, &text), None);
        assert_eq!(World::decode(500, 30, &text), None);
        assert_eq!(World::decode(48, 30, "not base64 at all!"), None);
    }

    #[test]
    fn every_size_fits_and_neighbours_stay_on_the_board() {
        for name in ["small", "medium", "large", "huge"] {
            let (w, h) = size_for(name);
            assert!(valid_size(w, h));
            let world = World::generate(w, h, 3);
            assert_eq!(world.neighbours(0).count(), 2);
            assert_eq!(world.neighbours(world.idx(h - 1, w - 1)).count(), 2);
            assert_eq!(world.neighbours(world.idx(5, 5)).count(), 4);
        }
    }
}
