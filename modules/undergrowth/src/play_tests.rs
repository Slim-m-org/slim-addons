use super::*;
use crate::fake::FakeHost;
use serde_json::json;

const T0: i64 = 1_700_000_000_000;

/// Runs one request through the real handler, as slim would.
pub fn call(host: &FakeHost, command: &str, caller: &str, input: &str, now: i64, settings: Value) -> Value {
    host.next_run();
    let req = json!({"command": command, "input": input, "caller": {"id": caller}, "entropy": format!("e{now}{caller}"),
        "now_ms": now, "settings": settings});
    serde_json::from_slice(&crate::handle(host, req.to_string().as_bytes())).unwrap()
}

fn scene(reply: &Value) -> Value {
    assert_eq!(reply["ok"], true, "{reply}");
    serde_json::from_str(reply["output"].as_str().unwrap()).unwrap()
}

fn press(host: &FakeHost, caller: &str, action: &str, state: &str, now: i64) -> Value {
    let input = json!({"action": action, "state": state}).to_string();
    call(host, "undergrowth", caller, &input, now, json!({}))
}

fn state(s: &Value) -> String {
    s["state"].as_str().unwrap().to_owned()
}

fn decoded(s: &Value) -> (Game, World) {
    Game::decode(&state(s)).expect("a state this module wrote")
}

fn launch(host: &FakeHost) -> String {
    state(&scene(&call(host, "undergrowth", "alice", "", T0, json!({}))))
}

fn link_code(panel: &Value) -> String {
    let text = panel["ops"].to_string();
    let at = text.find("Post ug-").expect("a code is shown");
    text[at + 5..at + 14].to_owned()
}

fn message(id: &str, author: &str, content: &str) -> String {
    json!({"event": "message.created", "id": id, "at_ms": T0,
        "data": {"message_id": id, "channel_id": "c", "author_id": author, "content": content, "is_bot": false}})
    .to_string()
}

#[test]
fn a_launch_deals_soil_and_asks_to_be_redrawn() {
    let host = FakeHost::default();
    let s = scene(&call(&host, "undergrowth", "alice", "", T0, json!({})));
    assert_eq!(s["$slim"], "scene/1");
    assert_eq!(s["refresh_ms"], 3000);
    assert_eq!(s["controls"], json!(["play", "join", "song", "my colony"]));
    let grid = s["ops"].as_array().unwrap().iter().find(|o| o["op"] == "cells").unwrap();
    assert_eq!((grid["cols"].as_u64(), grid["rows"].as_u64()), (Some(64), Some(40)));
    assert_eq!((grid["tap"].as_str(), grid["alt"].as_str(), grid["tap_batch"].as_bool()), (Some("lure"), Some("fruit"), Some(true)));
    let (game, _) = decoded(&s);
    assert_eq!((game.season, game.tick, game.ms), (1, 0, T0));
}

#[test]
fn the_clock_moves_the_world_by_the_time_that_passed() {
    let host = FakeHost::default();
    let s = scene(&call(&host, "undergrowth", "alice", "", T0, json!({})));
    let s = scene(&press(&host, "alice", "refresh", &state(&s), T0 + 1_000));
    assert_eq!(decoded(&s).0.tick, 2, "two whole 400 ms ticks in a second");
    let s = scene(&press(&host, "alice", "refresh", &state(&s), T0 + 1_100));
    assert_eq!(decoded(&s).0.tick, 2, "the 200 ms left over is kept, not lost");
    let s = scene(&press(&host, "alice", "refresh", &state(&s), T0 + 1_200));
    assert_eq!(decoded(&s).0.tick, 3);
    let s = scene(&press(&host, "alice", "refresh", &state(&s), T0 + 3_600_000));
    let (game, _) = decoded(&s);
    assert_eq!((game.tick, game.ms), (3 + MAX_CATCH_UP as u32, T0 + 3_600_000), "an hour away wakes in one bounded step");
}

#[test]
fn ticks_owed_keeps_the_remainder_and_ignores_a_clock_that_ran_backwards() {
    assert_eq!(ticks_owed(1000, 1900, 400), (2, 1800));
    assert_eq!(ticks_owed(1000, 900, 400), (0, 1000));
    assert_eq!(ticks_owed(0, 1_000_000, 400), (MAX_CATCH_UP as u32, 1_000_000));
}

#[test]
fn two_presses_from_the_same_frame_both_count_whichever_is_stored() {
    let host = FakeHost::default();
    let s0 = launch(&host);
    // Both press on the same stored frame; slim keeps whichever frame lands last.
    let a = scene(&press(&host, "alice", "join", &s0, T0 + 10));
    let b = scene(&press(&host, "bob", "join", &s0, T0 + 10));
    for stored in [a, b] {
        let next = scene(&press(&host, "carol", "refresh", &state(&stored), T0 + 20));
        let (game, _) = decoded(&next);
        let names: Vec<&str> = game.c.iter().map(|c| c.pid.as_str()).collect();
        assert!(names.contains(&"alice") && names.contains(&"bob"), "{names:?}");
        assert_eq!(game.c.len(), 2, "nobody joined twice");
    }
}

#[test]
fn without_the_log_a_press_still_counts_but_says_it_can_be_lost() {
    let host = FakeHost::default();
    let s0 = launch(&host);
    host.refuse_data.set(true);
    let s = scene(&press(&host, "alice", "join", &s0, T0 + 10));
    assert_eq!(decoded(&s).0.c.len(), 1);
    let s = scene(&press(&host, "alice", "refresh", &state(&s), T0 + 20));
    assert!(s["status"].as_str().unwrap().contains("could not be saved"), "{}", s["status"]);
}

#[test]
fn the_presser_hears_their_own_move_and_a_refused_one_is_silent() {
    let host = FakeHost::default();
    let s0 = launch(&host);
    let s = scene(&press(&host, "alice", "join", &s0, T0 + 10));
    assert!(s["ops"].as_array().unwrap().iter().any(|o| o["op"] == "notes"), "joining plays a cue");
    let s = scene(&press(&host, "alice", "fruit:0,0", &state(&s), T0 + 20));
    assert!(s["ops"].as_array().unwrap().iter().all(|o| o["op"] != "notes"));
    assert!(s["status"].as_str().unwrap().contains("own hyphae"));
}

#[test]
fn the_song_sweeps_a_playhead_and_sings() {
    let host = FakeHost::default();
    let s0 = launch(&host);
    let s = scene(&press(&host, "alice", "song", &s0, T0 + 10));
    let ops = s["ops"].as_array().unwrap();
    assert!(ops.iter().any(|o| o["op"] == "line" && o["sweep"]["dx"].as_u64() == Some(64)));
    assert!(ops.iter().any(|o| o["op"] == "notes"));
    assert!(ops.iter().filter(|o| o.get("sweep").is_some()).count() <= 8);
}

#[test]
fn weather_from_the_inbox_reaches_every_game_once() {
    let host = FakeHost::default();
    let s0 = launch(&host);
    for k in 0..3 {
        let said = call(&host, "@event", "bot", &message(&format!("m{k}"), "u1", "hello there"), T0, json!({}));
        assert_eq!(said["ok"], true);
    }
    let s = scene(&press(&host, "alice", "refresh", &s0, T0 + 10));
    assert_eq!(decoded(&s).0.leaves, 3);
    let s = scene(&press(&host, "alice", "refresh", &state(&s), T0 + 20));
    assert_eq!(decoded(&s).0.leaves, 3, "a leaf is taken in once");
    let late = state(&scene(&call(&host, "undergrowth", "bob", "", T0 + 30, json!({}))));
    let s = scene(&press(&host, "bob", "refresh", &late, T0 + 40));
    assert_eq!(decoded(&s).0.leaves, 0, "a new game starts after the weather before it");
}

#[test]
fn a_season_ends_once_records_everyone_and_asks_for_one_herald() {
    let host = FakeHost::default();
    let fast = json!({"season_ticks": 200, "tick_ms": 150});
    let frame = |who: &str, action: &str, st: &str, now: i64| {
        call(&host, "undergrowth", who, &json!({"action": action, "state": st}).to_string(), now, fast.clone())
    };
    let mut s = scene(&call(&host, "undergrowth", "alice", "", T0, fast.clone()));
    let mut now = T0;
    for who in ["alice", "bob"] {
        now += 10;
        s = scene(&frame(who, "join", &state(&s), now));
    }
    let mut timers = Vec::new();
    let mut before_end = state(&s);
    while !decoded(&s).0.over {
        before_end = state(&s);
        now += 6_000;
        let reply = frame("alice", "refresh", &state(&s), now);
        timers.extend(reply.get("run_after").cloned());
        s = scene(&reply);
    }
    assert!(s.get("refresh_ms").is_none(), "a finished season stops asking to be redrawn");
    assert_eq!(s["controls"], json!(["new season", "song", "my colony"]));
    let again = frame("bob", "refresh", &before_end, now + 1);
    assert!(again.get("run_after").is_none(), "a second frame that saw the same end records nothing");
    assert_eq!(timers.len(), 1);
    assert_eq!(timers[0]["command"], "herald");
    let game_id = timers[0]["input"].as_str().unwrap().to_owned();
    let players = host.rows("players");
    assert_eq!(players.len(), 2);
    assert_eq!(players.iter().map(|p| p["wins"].as_i64().unwrap()).sum::<i64>(), 1);
    assert!(players.iter().all(|p| p["seasons"] == 1));
    let posted = call(&host, "herald", "bot", &game_id, now + 10_000, json!({}));
    assert!(posted["output"].as_str().unwrap().starts_with("Season 1 of the undergrowth is over."), "{posted}");
    let twice = call(&host, "herald", "bot", &game_id, now + 20_000, json!({}));
    assert_eq!(twice["output"], "nothing new to announce");
    assert_eq!(host.posts.borrow().len(), 1);
    let next = scene(&frame("bob", "new season", &state(&s), now + 30_000));
    let (game, _) = decoded(&next);
    assert_eq!((game.season, game.tick, game.over, game.c.len()), (2, 0, false, 2), "both players root again");
}

#[test]
fn my_colony_is_private_and_a_burst_from_it_lands_on_the_board_once() {
    let host = FakeHost::default();
    let s0 = launch(&host);
    let stranger = scene(&press(&host, "zed", "my colony", &s0, T0 + 5));
    assert_eq!(stranger["private"], true);
    assert_eq!(stranger["controls"], json!(["back", "join"]));
    let shared = state(&scene(&press(&host, "alice", "join", &s0, T0 + 10)));
    let panel = scene(&press(&host, "alice", "my colony", &shared, T0 + 20));
    assert_eq!(panel["private"], true);
    assert_eq!(panel["controls"], json!(["back", "burst", "spore shot", "link profile"]));
    assert!(panel["ops"].as_array().unwrap().iter().any(|o| o["op"] == "image"), "a portrait");
    assert_eq!(panel["state"], json!(shared), "a private frame hands back the board it was shown");
    let burst = scene(&press(&host, "alice", "burst", &shared, T0 + 30));
    assert_eq!(burst["status"], "your colony surges for thirty ticks");
    let named = scene(&press(&host, "alice", "name:Moss Lord", &shared, T0 + 40));
    assert_eq!(named["status"], "your colony is now Moss Lord");
    let board = scene(&press(&host, "alice", "back", &shared, T0 + 50));
    let (game, _) = decoded(&board);
    assert_eq!(game.c[0].name, "Moss Lord");
    assert!(game.c[0].burst > 0);
    assert_eq!(game.c[0].energy, 0, "thirty for the burst, once");
}

#[test]
fn a_linked_profile_shows_its_record_and_the_champion_badge() {
    let host = FakeHost::default();
    let shared = state(&scene(&press(&host, "alice", "join", &launch(&host), T0 + 10)));
    let code = link_code(&scene(&press(&host, "alice", "link profile", &shared, T0 + 20)));
    let said = call(&host, "@event", "bot", &message("m9", "user-a", &format!("linking {code}")), T0 + 60_000, json!({}));
    assert!(said["output"].as_str().unwrap().contains("linked"), "{said}");
    let pid = crate::colony::pid_of("alice");
    let fact = json!({"_id": 999, "name": "champion", "val": pid, "num": 1});
    host.tables.borrow_mut().get_mut("facts").unwrap().push(fact.as_object().unwrap().clone());
    let slot = |kind: &str, name: &str| -> Value {
        let ask = json!({"slot": kind, "name": name, "user_ids": ["user-a", "user-b"], "viewer_id": "v"});
        serde_json::from_str(call(&host, "@slot", "v", &ask.to_string(), T0, json!({}))["output"].as_str().unwrap()).unwrap()
    };
    let section = slot("profile.section", "grove");
    assert_eq!(section["$slim"], "slot/1");
    assert!(section["items"]["user-a"]["fallback"].as_str().unwrap().contains("1 leaf"));
    assert!(section["items"]["user-a"].to_string().contains("Spore champion"));
    assert!(section["items"].get("user-b").is_none());
    let badge = slot("member.badge", "champion");
    assert_eq!(badge["items"]["user-a"]["nodes"][0]["text"], "Spore champion");
    assert!(badge["items"].get("user-b").is_none());
}

#[test]
fn a_stale_or_reused_link_code_links_nothing() {
    let host = FakeHost::default();
    let shared = state(&scene(&press(&host, "alice", "join", &launch(&host), T0 + 10)));
    let code = link_code(&scene(&press(&host, "alice", "link profile", &shared, T0 + 20)));
    let late = call(&host, "@event", "bot", &message("m1", "user-a", &code), T0 + 20 + crate::LINK_TTL_MS + 1, json!({}));
    assert!(!late["output"].as_str().unwrap().contains("linked"));
    let reused = call(&host, "@event", "bot", &message("m2", "user-b", &code), T0 + 30, json!({}));
    assert!(!reused["output"].as_str().unwrap().contains("linked"), "a code is used up when read");
    assert!(host.rows("players").iter().all(|p| p["user_id"].is_null()));
}

#[test]
fn calls_warm_the_soil_exactly_once_even_when_the_event_is_missed() {
    let host = FakeHost::default();
    let session = |id: i64, user: &str, mins: i64| {
        json!({"id": id, "user_id": user, "channel_id": "c", "joined_at_ms": 0, "left_at_ms": mins * 60_000, "alone_ms": 0})
    };
    host.log.borrow_mut().push(session(1, "u1", 30));
    host.log.borrow_mut().push(session(2, "u2", 0));
    let closed = json!({"event": "voice.session.closed", "id": "v", "at_ms": T0, "data": {}}).to_string();
    call(&host, "@event", "bot", &closed, T0, json!({}));
    call(&host, "@event", "bot", &closed, T0, json!({}));
    host.log.borrow_mut().push(session(3, "u1", 15));
    // No event for the third call: the daily almanac's catch-up still finds it.
    call(&host, "almanac", "bot", "", T0 + 1, json!({"almanac": false}));
    let warm: Vec<i64> = host.rows("inbox").iter().filter(|r| r["kind"] == "warm").map(|r| r["amount"].as_i64().unwrap()).collect();
    assert_eq!(warm, vec![30, 15]);
    let u1 = host.rows("feeders").into_iter().find(|r| r["user_id"] == "u1").unwrap();
    assert_eq!(u1["warm_min"], 45);
}

#[test]
fn the_almanac_posts_once_a_day_and_tidies_old_rows() {
    let host = FakeHost::default();
    let day = 86_400_000;
    call(&host, "@event", "bot", &message("m1", "u1", "hi"), T0 - 4 * day, json!({}));
    call(&host, "@event", "bot", &message("m2", "u1", "hi"), T0, json!({}));
    let first = call(&host, "almanac", "bot", "", T0 + 1000, json!({}));
    assert!(first["output"].as_str().unwrap().starts_with("The undergrowth, last 24 hours: 1 leaf fell"), "{first}");
    assert_eq!(host.rows("inbox").len(), 1, "the four-day-old leaf was tidied");
    let second = call(&host, "almanac", "bot", "", T0 + 2000, json!({}));
    assert_eq!(second["output"], "today's almanac is already out");
    assert_eq!(host.posts.borrow().len(), 1);
}

#[test]
fn bots_drop_no_leaves_and_a_quiet_day_posts_nothing() {
    let host = FakeHost::default();
    let bot = json!({"event": "message.created", "id": "m", "at_ms": T0,
        "data": {"message_id": "m1", "channel_id": "c", "author_id": null, "content": "beep", "is_bot": true}})
    .to_string();
    call(&host, "@event", "bot", &bot, T0, json!({}));
    assert!(host.rows("inbox").is_empty());
    assert_eq!(call(&host, "almanac", "bot", "", T0, json!({}))["output"], "a quiet day; nothing to post");
}

#[test]
fn every_world_size_stays_under_the_stored_frame_ceiling_when_crowded() {
    for size in ["small", "medium", "large", "huge"] {
        let host = FakeHost::default();
        let settings = json!({"world": size});
        let mut s = scene(&call(&host, "undergrowth", "p0", "", T0, settings.clone()));
        for p in 0..6 {
            let input = json!({"action": "join", "state": state(&s)}).to_string();
            s = scene(&call(&host, "undergrowth", &format!("p{p}"), &input, T0 + 1, settings.clone()));
        }
        let (mut game, mut world) = decoded(&s);
        for c in &mut game.c {
            c.waypoints = (0..8).map(|k| k * 37).collect();
        }
        sim::advance(&mut game, &mut world, 400);
        let owned: Vec<usize> = (0..world.cells.len()).filter(|i| world.cells[*i].owner() != 0).collect();
        for i in owned.into_iter().take(48) {
            world.cells[i].set_mushroom(true);
        }
        let crowded = game.encode(&world);
        let reply = call(&host, "undergrowth", "p0", &json!({"action": "song", "state": crowded}).to_string(), T0 + 2, settings.clone());
        let out = reply["output"].as_str().unwrap();
        assert!(out.len() < 64 * 1024, "{size}: {} bytes", out.len());
        let panel = call(&host, "undergrowth", "p0", &json!({"action": "my colony", "state": crowded}).to_string(), T0 + 3, settings);
        assert!(panel["output"].as_str().unwrap().len() < 64 * 1024, "{size} panel");
    }
}

#[test]
fn hostile_input_never_breaks_a_frame() {
    let host = FakeHost::default();
    let inputs = ["", "not json", "{}", r#"{"action":"lure:1,1"}"#, r#"{"action":"lure:1,1","state":"|||"}"#, r#"{"action":"x","state":42}"#];
    for input in inputs {
        let reply = call(&host, "undergrowth", "alice", input, T0, json!({}));
        scene(&reply);
    }
    assert_eq!(call(&host, "nope", "a", "", T0, json!({}))["ok"], false);
    assert_eq!(call(&host, "@event", "a", "{", T0, json!({}))["ok"], false);
    assert_eq!(call(&host, "@slot", "a", "{", T0, json!({}))["ok"], false);
}

#[test]
fn a_malformed_action_on_a_real_board_redraws_it_and_resets_nothing() {
    let host = FakeHost::default();
    let mut s = launch(&host);
    for who in ["alice", "bob"] {
        s = state(&scene(&press(&host, who, "join", &s, T0 + 10)));
    }
    let (before, _) = Game::decode(&s).unwrap();
    for action in [json!(5), json!(null), json!({"x": 1}), json!("")] {
        let input = json!({"action": action, "state": s}).to_string();
        let (after, _) = decoded(&scene(&call(&host, "undergrowth", "mallory", &input, T0 + 20, json!({}))));
        assert_eq!((&after.g, after.season, after.seed), (&before.g, before.season, before.seed), "{action}");
        let pids = |g: &Game| g.c.iter().map(|c| c.pid.clone()).collect::<Vec<_>>();
        assert_eq!(pids(&after), pids(&before), "{action}");
    }
    let (bare, _) = decoded(&scene(&call(&host, "undergrowth", "mallory", &json!({"state": s}).to_string(), T0 + 20, json!({}))));
    assert_eq!((bare.g, bare.c.len()), (before.g, 2), "a press with a board and no action keeps it too");
}

/// Two players join and the clock runs until the season ends; the finished frame and the time.
fn play_to_the_end(host: &FakeHost, fast: &Value) -> (Value, i64) {
    let mut s = scene(&call(host, "undergrowth", "alice", "", T0, fast.clone()));
    let mut now = T0;
    for who in ["alice", "bob"] {
        now += 10;
        s = scene(&call(host, "undergrowth", who, &json!({"action": "join", "state": state(&s)}).to_string(), now, fast.clone()));
    }
    while !decoded(&s).0.over {
        now += 6_000;
        s = scene(&call(host, "undergrowth", "alice", &json!({"action": "refresh", "state": state(&s)}).to_string(), now, fast.clone()));
    }
    (s, now)
}

#[test]
fn a_new_season_is_a_move_so_concurrent_frames_deal_the_same_soil() {
    let host = FakeHost::default();
    let fast = json!({"season_ticks": 200, "tick_ms": 150});
    let (over, now) = play_to_the_end(&host, &fast);
    let frame = |who: &str, action: &str, st: &str, at: i64| {
        scene(&call(&host, "undergrowth", who, &json!({"action": action, "state": st}).to_string(), at, fast.clone()))
    };
    let song = frame("carol", "song", &state(&over), now + 100);
    let a = frame("alice", "new season", &state(&over), now + 100);
    let b = frame("bob", "new season", &state(&over), now + 100);
    let (ga, wa) = decoded(&a);
    let (gb, wb) = decoded(&b);
    assert_eq!((ga.season, gb.season), (2, 2), "the second press finds the season already begun");
    assert_eq!((ga.seed, &wa), (gb.seed, &wb), "both frames dealt the same soil");
    assert!(b["status"].as_str().unwrap().contains("already begun"), "{}", b["status"]);
    let (gs, _) = decoded(&song);
    assert!(gs.over && gs.season == 1, "a song that ran before the new season still shows the finished board");
    // Whichever frame slim stored, even the older finished one, carol's join on it lands on the new season.
    for stored in [&a, &b, &song] {
        let joined = frame("carol", "join", &state(stored), now + 200);
        let (g, _) = decoded(&joined);
        assert_eq!((g.season, g.seed, g.over), (2, ga.seed, false));
        assert!(g.colony("carol").is_some(), "{}", joined["status"]);
    }
}

#[test]
fn the_season_end_claim_fixes_the_standings_the_herald_posts() {
    let host = FakeHost::default();
    let fast = json!({"season_ticks": 200, "tick_ms": 150});
    let (_, now) = play_to_the_end(&host, &fast);
    let row = || host.rows("games").into_iter().next().unwrap();
    let claimed = row()["standings"].clone();
    assert_eq!(row()["ended"], 1);
    let store = Store::new(&host);
    let game_id = row()["game"].as_str().unwrap().to_owned();
    let late = r#"{"season":1,"over":false,"colonies":[{"name":"Late Frame","total":1}]}"#;
    host.next_run();
    store.save_standings(&game_id, (1, 150, 200), late, now + 1).unwrap();
    assert!(!store.claim_season_end(&game_id, (1, 200, 200), late, now + 2).unwrap());
    assert_eq!(row()["standings"], claimed, "a frame that lost the race writes nothing over the final standings");
    let posted = call(&host, "herald", "bot", &game_id, now + 10_000, json!({}));
    assert!(!posted["output"].as_str().unwrap().contains("Late Frame"), "{posted}");
}
