use super::*;
use crate::world::{World, LOAM, WILD};

fn fresh() -> (Game, World) {
    let world = World::generate(64, 40, 11);
    let game = Game { g: "g".into(), seed: 11, season: 1, end: 500, w: 64, h: 40, ..Game::default() };
    (game, world)
}

fn act(game: &mut Game, world: &mut World, pid: &str, act: &str, arg: &str) -> Said {
    apply(game, world, &Move { pid: pid.into(), act: act.into(), arg: arg.into() })
}

#[test]
fn six_players_root_far_apart_and_a_seventh_waits() {
    let (mut game, mut world) = fresh();
    for p in 0..6 {
        assert!(act(&mut game, &mut world, &format!("p{p}"), "join", "").is_ok());
    }
    let refused = act(&mut game, &mut world, "p6", "join", "").unwrap_err();
    assert!(refused.contains("six colonies"), "{refused}");
    let again = act(&mut game, &mut world, "p0", "join", "").unwrap_err();
    assert!(again.contains("already growing"), "{again}");
    let hearts: Vec<usize> = game.c.iter().map(|c| c.heart as usize).collect();
    for (k, a) in hearts.iter().enumerate() {
        assert_eq!(world.cells[*a].owner(), game.c[k].slot);
        assert!(world.cells[*a].anchor());
        for b in hearts.iter().skip(k + 1) {
            let d = (a / 64).abs_diff(b / 64) + (a % 64).abs_diff(b % 64);
            assert!(d >= 4, "hearts {a} and {b} are {d} apart");
        }
    }
    let slots: std::collections::HashSet<u8> = game.c.iter().map(|c| c.slot).collect();
    assert_eq!(slots.len(), 6);
}

#[test]
fn a_nameless_caller_cannot_join() {
    let (mut game, mut world) = fresh();
    assert!(act(&mut game, &mut world, "", "join", "").is_err());
}

#[test]
fn a_lure_lays_waypoints_and_a_long_drag_keeps_its_shape() {
    let (mut game, mut world) = fresh();
    act(&mut game, &mut world, "a", "join", "").unwrap();
    assert!(act(&mut game, &mut world, "a", "lure", "10,10").is_ok());
    assert_eq!(game.c[0].waypoints, vec![world.idx(10, 10) as u32]);
    let drag: Vec<String> = (0..30).map(|c| format!("20,{c}")).collect();
    let said = act(&mut game, &mut world, "a", "lure", &drag.join(";")).unwrap();
    assert!(said.contains("8 waypoints"), "{said}");
    assert_eq!(game.c[0].waypoints.first(), Some(&(world.idx(20, 0) as u32)));
    assert_eq!(game.c[0].waypoints.last(), Some(&(world.idx(20, 29) as u32)));
    assert!(act(&mut game, &mut world, "a", "lure", "999,999").is_err());
    assert!(act(&mut game, &mut world, "a", "lure", "nonsense").is_err());
    assert!(act(&mut game, &mut world, "stranger", "lure", "1,1").is_err());
}

#[test]
fn fruiting_needs_your_own_hyphae_and_enough_energy() {
    let (mut game, mut world) = fresh();
    act(&mut game, &mut world, "a", "join", "").unwrap();
    let heart = game.c[0].heart as usize;
    let (r, c) = (heart / 64, heart % 64);
    let at = format!("{r},{c}");
    let poor = act(&mut game, &mut world, "a", "fruit", &at).unwrap_err();
    assert!(poor.contains("40 energy"), "{poor}");
    game.c[0].energy = 200;
    assert!(act(&mut game, &mut world, "a", "fruit", &at).is_ok());
    assert!(world.cells[heart].mushroom());
    assert_eq!(game.c[0].energy, 160);
    assert!(act(&mut game, &mut world, "a", "fruit", &at).unwrap_err().contains("already"));
    assert!(act(&mut game, &mut world, "a", "fruit", "39,0").unwrap_err().contains("own hyphae"));
    assert_eq!(game.fx.last().map(|f| f.k), Some('f'));
}

#[test]
fn no_more_than_eight_mushrooms_stand_for_one_colony() {
    let (mut game, mut world) = fresh();
    act(&mut game, &mut world, "a", "join", "").unwrap();
    game.c[0].energy = 240;
    for c in 0..9 {
        let i = world.idx(30, c);
        world.cells[i].set_terrain(LOAM);
        world.cells[i].set_owner(1);
        if c < 8 {
            world.cells[i].set_mushroom(true);
        }
    }
    let refused = act(&mut game, &mut world, "a", "fruit", "30,8").unwrap_err();
    assert!(refused.contains("eight"), "{refused}");
}

#[test]
fn burst_and_spore_shot_spend_energy() {
    let (mut game, mut world) = fresh();
    act(&mut game, &mut world, "a", "join", "").unwrap();
    game.c[0].energy = 10;
    assert!(act(&mut game, &mut world, "a", "burst", "").is_err());
    game.c[0].energy = 100;
    assert!(act(&mut game, &mut world, "a", "burst", "").is_ok());
    assert_eq!((game.c[0].energy, game.c[0].burst), (70, 30));
    assert!(act(&mut game, &mut world, "a", "spore", "").unwrap_err().contains("lure"));
    let target = (0..world.cells.len()).rev().find(|i| world.cells[*i].passable() && world.cells[*i].owner() == 0).unwrap();
    let (r, c) = (target / 64, target % 64);
    act(&mut game, &mut world, "a", "lure", &format!("{r},{c}")).unwrap();
    game.c[0].energy = 20;
    assert!(act(&mut game, &mut world, "a", "spore", "").unwrap_err().contains("60 energy"));
    game.c[0].energy = 100;
    assert!(act(&mut game, &mut world, "a", "spore", "").is_ok());
    assert_eq!(world.cells[target].owner(), 1);
    assert!(world.cells[target].anchor());
    assert_eq!(game.c[0].energy, 40);
}

#[test]
fn a_name_is_cleaned_and_a_finished_season_still_takes_one() {
    let (mut game, mut world) = fresh();
    act(&mut game, &mut world, "a", "join", "").unwrap();
    assert!(act(&mut game, &mut world, "a", "name", "  Moss   Lord  ").is_ok());
    assert_eq!(game.c[0].name, "Moss Lord");
    assert!(act(&mut game, &mut world, "a", "name", "<<>>").is_err());
    game.over = true;
    assert!(act(&mut game, &mut world, "a", "name", "Late").is_ok());
    assert!(act(&mut game, &mut world, "a", "burst", "").unwrap_err().contains("over"));
}

#[test]
fn leaves_feed_the_topsoil_and_rain_and_warmth_are_kept() {
    let (mut game, mut world) = fresh();
    let i = world.idx(3, 20);
    let before = world.cells[i].nutrient();
    weather(&mut game, &mut world, &Weather { kind: "leaf".into(), at: 20 + 64 * 5, amount: 3 });
    assert!(world.cells[i].nutrient() > before || before == 15 || !world.cells[i].passable());
    assert_eq!(game.leaves, 1);
    weather(&mut game, &mut world, &Weather { kind: "rain".into(), at: 7, amount: 1 });
    weather(&mut game, &mut world, &Weather { kind: "rain".into(), at: 7, amount: 1 });
    assert_eq!(game.rain, vec![(7, 40)]);
    weather(&mut game, &mut world, &Weather { kind: "warm".into(), at: 0, amount: 1000 });
    assert_eq!(game.warm, 360);
    weather(&mut game, &mut world, &Weather { kind: "hail".into(), at: 0, amount: 1 });
    assert!(game.fx.iter().any(|f| f.k == 'l') && game.fx.iter().any(|f| f.k == 'r'));
}

#[test]
fn the_wild_rot_wakes_once_and_then_only_grows_stronger() {
    let (mut game, mut world) = fresh();
    weather(&mut game, &mut world, &Weather { kind: "wild".into(), at: 1, amount: 1 });
    weather(&mut game, &mut world, &Weather { kind: "wild".into(), at: 2, amount: 2 });
    let wild: Vec<_> = game.c.iter().filter(|c| c.slot == WILD).collect();
    assert_eq!(wild.len(), 1);
    assert_eq!(wild[0].energy, 100);
    assert!(wild[0].heart as usize / 64 >= 20, "the wild rot wakes deep");
}

#[test]
fn cells_of_reads_rows_and_columns_inside_the_board() {
    let (_, world) = fresh();
    assert_eq!(cells_of(&world, "0,0;39,63; 1 , 2 ;40,0;0,64;x"), vec![0, (39 * 64 + 63) as u32, (64 + 2) as u32]);
    assert!(cells_of(&world, "").is_empty());
}
