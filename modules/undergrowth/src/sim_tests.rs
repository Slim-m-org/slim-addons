use super::*;
use crate::colony::Colony;
use crate::moves::{self, Move, Weather};
use crate::world::{World, WILD};

fn season(w: usize, h: usize, players: usize) -> (Game, World) {
    let mut world = World::generate(w, h, 77);
    let mut game = Game { g: "g".into(), seed: 77, season: 1, end: 2400, w, h, ..Game::default() };
    for p in 0..players {
        let said = moves::apply(&mut game, &mut world, &Move { pid: format!("p{p}"), act: "join".into(), arg: String::new() });
        assert!(said.is_ok(), "{said:?}");
    }
    (game, world)
}

fn owned(world: &World, slot: u8) -> usize {
    world.cells.iter().filter(|c| c.owner() == slot).count()
}

#[test]
fn colonies_grow_but_do_not_swallow_the_soil_in_the_first_minutes() {
    let (mut game, mut world) = season(64, 40, 4);
    let start: Vec<usize> = (1..=4).map(|s| owned(&world, s)).collect();
    advance(&mut game, &mut world, 150);
    let passable = world.cells.iter().filter(|c| c.passable()).count();
    for slot in 1..=4u8 {
        let now = owned(&world, slot);
        assert!(now > start[slot as usize - 1] + 10, "slot {slot} only reached {now}");
        assert!(now < passable / 3, "slot {slot} took {now} of {passable} in 150 ticks");
    }
}

#[test]
fn a_whole_season_runs_to_its_end_and_ranks_everyone() {
    let (mut game, mut world) = season(64, 40, 6);
    moves::weather(&mut game, &mut world, &Weather { kind: "wild".into(), at: 9, amount: 9 });
    let mut fights = 0;
    while !game.over {
        let news = advance(&mut game, &mut world, 40);
        fights += news.took.iter().sum::<u32>();
    }
    assert_eq!(game.tick, game.end);
    assert!(fights > 0, "nobody ever took a rival's cell");
    let ranking = game.ranking();
    assert_eq!(ranking.len(), 6);
    assert!(ranking.windows(2).all(|p| p[0].total() >= p[1].total()));
    assert!(game.c.iter().all(|c| world.cells[c.heart as usize].owner() == c.slot), "a heart fell");
}

#[test]
fn a_lure_pulls_growth_toward_it() {
    let (mut game, mut world) = season(64, 40, 1);
    let heart = game.c[0].heart as usize;
    let (hr, hc) = (heart / 64, heart % 64);
    let target_c = if hc < 32 { 60 } else { 3 };
    let target = world.idx(hr.max(4), target_c) as u32;
    game.c[0].waypoints = vec![target];
    game.c[0].energy = 200;
    advance(&mut game, &mut world, 120);
    let reach = (0..world.cells.len())
        .filter(|i| world.cells[*i].owner() == 1)
        .map(|i| (i % 64) as i32 - hc as i32)
        .map(|d| if target_c > hc { d } else { -d })
        .max()
        .unwrap();
    assert!(reach > 12, "only reached {reach} columns toward the lure");
}

#[test]
fn hyphae_cut_off_from_every_anchor_wither_and_enrich_the_soil() {
    let (mut game, mut world) = season(48, 30, 1);
    let far = world.idx(25, 40);
    world.cells[far].set_terrain(crate::world::LOAM);
    world.cells[far].set_owner(1);
    world.cells[far].set_nutrient(3);
    let mut news = News::default();
    wither(&mut game, &mut world, &mut news);
    assert_eq!(world.cells[far].owner(), 0);
    assert!(world.cells[far].dead());
    assert_eq!(world.cells[far].nutrient(), 5);
    assert_eq!(news.withered[1], 1);
    assert_eq!(world.cells[game.c[0].heart as usize].owner(), 1, "the heart stays");
}

#[test]
fn an_outpost_keeps_its_own_hyphae_alive() {
    let (mut game, mut world) = season(48, 30, 1);
    let post = world.idx(25, 40);
    let next = world.idx(25, 41);
    for i in [post, next] {
        world.cells[i].set_terrain(crate::world::LOAM);
        world.cells[i].set_owner(1);
    }
    world.cells[post].set_anchor(true);
    wither(&mut game, &mut world, &mut News::default());
    assert_eq!(world.cells[next].owner(), 1);
}

#[test]
fn the_same_season_replays_the_same_way() {
    let (mut a, mut wa) = season(48, 30, 3);
    let (mut b, mut wb) = season(48, 30, 3);
    advance(&mut a, &mut wa, 300);
    advance(&mut b, &mut wb, 300);
    assert_eq!(wa, wb);
    assert_eq!(a, b);
}

#[test]
fn the_wild_rot_plays_itself() {
    let (mut game, mut world) = season(64, 40, 0);
    moves::weather(&mut game, &mut world, &Weather { kind: "wild".into(), at: 3, amount: 3 });
    assert_eq!(game.c.len(), 1);
    let before = owned(&world, WILD);
    advance(&mut game, &mut world, 200);
    assert!(owned(&world, WILD) > before + 10);
    assert!(game.ranking().is_empty(), "the wild rot never ranks");
}

#[test]
fn a_mushroom_scores_and_throws_spores() {
    let (mut game, mut world) = season(64, 40, 1);
    let heart = game.c[0].heart as usize;
    world.cells[heart].set_mushroom(true);
    let mut spores = 0;
    for _ in 0..4 {
        spores += advance(&mut game, &mut world, 50).spores[1];
    }
    assert!(game.c[0].score >= 15, "score {}", game.c[0].score);
    assert!(spores >= 1);
}

#[test]
fn orphaned_cells_of_a_vanished_colony_are_cleared() {
    let (mut game, mut world) = season(48, 30, 1);
    let at = world.idx(20, 20);
    world.cells[at].set_terrain(crate::world::LOAM);
    world.cells[at].set_owner(5);
    game.c.push(Colony { pid: "x".into(), slot: 6, heart: 0, ..Colony::default() });
    advance(&mut game, &mut world, 1);
    assert_eq!(world.cells[at].owner(), 0);
}

/// Not a check: prints how a season unfolds, for tuning. `cargo test -- --ignored --nocapture season_curve`.
#[test]
#[ignore]
fn season_curve() {
    let (mut game, mut world) = season(64, 40, 4);
    for step in 0..24 {
        let news = advance(&mut game, &mut world, 100);
        let line: Vec<String> = game.c.iter().map(|c| format!("{}:{}c/{}e", c.slot, c.cells, c.energy)).collect();
        println!("t={} {} took={:?} withered={:?}", (step + 1) * 100, line.join(" "), &news.took[1..5], &news.withered[1..5]);
    }
}
