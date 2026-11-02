//! Word Guess: one word a day, the same for everyone, played on a private board.
//!
//! The app message is shared, so its own frame is only a landing with nothing of anyone's board on
//! it. A player's board comes back as a private scene, which the host shows to them alone and does
//! not store or broadcast. Whatever state the client sends is ignored for the daily puzzle: the
//! player's guesses live in the module's database, keyed by the opaque id the host gives it, so
//! a board cannot be forged or replayed to play a day twice.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::daily::{answer_for, day_of, Plays, Stored};
use crate::game::{Game, TRIES};
use crate::host::Host;
use crate::scene::{render_board, render_landing, share_text, Board};
use crate::words::WORDS;

/// What the host tells a command about the run, beyond who is asking.
pub struct Run<'a> {
    pub caller: &'a str,
    pub now_ms: i64,
    pub entropy: &'a str,
}

pub fn apply<H: Host>(host: &H, command: &str, input: &str, run: &Run) -> Result<String, String> {
    match command {
        "wordguess" => Ok(play(host, input, run)),
        other => Err(format!("unknown command: {other}")),
    }
}

#[derive(Deserialize)]
struct Action {
    action: String,
}

/// Opaque ids are hex today; keep a stored key and a post from ever carrying anything else.
fn safe_id(caller: &str) -> String {
    caller.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(64).collect()
}

fn play<H: Host>(host: &H, input: &str, run: &Run) -> String {
    let day = day_of(run.now_ms);
    let player = safe_id(run.caller);
    let Ok(act) = serde_json::from_str::<Action>(input.trim()) else {
        return render_landing(day);
    };
    // The host hands a shared surface its stored frame's state, never the viewer's, so a private board keeps its own in storage.
    let name = act.action.as_str();
    let outcome = if let Some(word) = name.strip_prefix("guess:") {
        daily_guess(host, day, &player, word)
    } else if let Some(word) = name.strip_prefix("pguess:") {
        practice_guess(host, &player, word)
    } else {
        match name {
            "play today" | "today" | "my board" => daily_board(host, day, &player, None),
            "share" => share(host, day, &player),
            "practice" | "new practice" => new_practice(host, &player, practice_seed(run)),
            _ => return render_landing(day),
        }
    };
    outcome.unwrap_or_else(|why| failure(&why))
}

/// A private frame that says what went wrong, so a refused capability reads as a sentence, not a blank board.
fn failure(why: &str) -> String {
    let game = Game::new(WORDS[0]);
    let board = Board { game: &game, submit: "guess", controls: vec!["close"], status: why.to_owned(), state: String::new() };
    render_board(&board)
}

fn daily_guess<H: Host>(host: &H, day: i64, player: &str, word: &str) -> Result<String, String> {
    let plays = Plays::new(host);
    let stored = plays.load(day, player)?;
    let mut game = Game::with_guesses(answer_for(day), stored.guesses.iter().map(String::as_str));
    if game.guess(word) {
        let kept = Stored { guesses: game.guesses.clone(), done: game.over(), won: game.solved(), seed: 0 };
        plays.save(day, player, &kept)?;
    }
    daily_board(host, day, player, Some(game))
}

/// The player's board for today; `game` is the one just played, when there is one, so a refused guess can say why.
fn daily_board<H: Host>(host: &H, day: i64, player: &str, game: Option<Game>) -> Result<String, String> {
    let plays = Plays::new(host);
    let game = match game {
        Some(game) => game,
        None => {
            let stored = plays.load(day, player)?;
            Game::with_guesses(answer_for(day), stored.guesses.iter().map(String::as_str))
        }
    };
    let streak = plays.streak(player, day).unwrap_or(0);
    let status = if game.solved() {
        format!("got it in {}{}", game.guesses.len(), streak_text(streak))
    } else if game.over() {
        format!("out of tries - it was {}{}", game.answer.to_uppercase(), streak_text(streak))
    } else if let Some(why) = &game.refused {
        why.clone()
    } else {
        format!("{} of {TRIES} tries used{}", game.guesses.len(), streak_text(streak))
    };
    let controls = if game.over() { vec!["share", "practice", "close"] } else { vec!["practice", "close"] };
    Ok(render_board(&Board { game: &game, submit: "guess", controls, status, state: String::new() }))
}

fn streak_text(streak: i64) -> String {
    if streak > 1 {
        format!(" - {streak} days in a row")
    } else {
        String::new()
    }
}

/// Posts the day's result as the player, once they have finished it.
fn share<H: Host>(host: &H, day: i64, player: &str) -> Result<String, String> {
    let stored = Plays::new(host).load(day, player)?;
    let game = Game::with_guesses(answer_for(day), stored.guesses.iter().map(String::as_str));
    if !game.over() {
        return daily_board(host, day, player, None).map(|scene| with_status(&scene, "finish today's word before sharing it"));
    }
    let post = json!({"capability": "message.post", "content": share_text(day, &game)});
    let status = match host.call(&post) {
        Ok(_) => "posted to the channel".to_owned(),
        Err(why) if why.contains("not approved") => "An admin needs to approve Word Guess to post in the Dock.".to_owned(),
        Err(why) => format!("could not post ({why})"),
    };
    daily_board(host, day, player, None).map(|scene| with_status(&scene, &status))
}

fn with_status(scene: &str, status: &str) -> String {
    let mut value: Value = serde_json::from_str(scene).unwrap_or(Value::Null);
    value["status"] = json!(status);
    value.to_string()
}

/// A practice board's answer is derived from a seed the host's entropy picks.
fn practice_seed(run: &Run) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in run.entropy.as_bytes().iter().chain(run.caller.as_bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash | 1
}

fn practice_answer(seed: u64) -> &'static str {
    WORDS[((seed.wrapping_mul(6_364_136_223_846_793_005) ^ 0x9E37_79B9_7F4A_7C15) >> 17) as usize % WORDS.len()]
}

fn new_practice<H: Host>(host: &H, player: &str, seed: u64) -> Result<String, String> {
    Plays::new(host).save_practice(player, &Stored { seed, ..Stored::default() })?;
    Ok(practice_scene(&Game::new(practice_answer(seed))))
}

fn practice_guess<H: Host>(host: &H, player: &str, word: &str) -> Result<String, String> {
    let plays = Plays::new(host);
    let mut stored = plays.load_practice(player)?;
    if stored.seed == 0 {
        // Typed at a practice board that was never started: start one from what was typed, and play it.
        stored = Stored { seed: practice_seed(&Run { caller: player, now_ms: 0, entropy: word }), ..Stored::default() };
    }
    let mut game = Game::with_guesses(practice_answer(stored.seed), stored.guesses.iter().map(String::as_str));
    if game.guess(word) {
        plays.save_practice(player, &Stored { guesses: game.guesses.clone(), done: game.over(), won: game.solved(), seed: stored.seed })?;
    }
    Ok(practice_scene(&game))
}

fn practice_scene(game: &Game) -> String {
    let status = if game.solved() {
        format!("got it in {}", game.guesses.len())
    } else if game.over() {
        format!("out of tries - it was {}", game.answer.to_uppercase())
    } else if let Some(why) = &game.refused {
        why.clone()
    } else {
        format!("practice - {} of {TRIES} tries used", game.guesses.len())
    };
    render_board(&Board { game, submit: "pguess", controls: vec!["new practice", "today", "close"], status, state: String::new() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daily::DAY_MS;
    use crate::fake::FakeHost;

    const DAY: i64 = 20_000;
    const NOON: i64 = DAY * DAY_MS + DAY_MS / 2;

    fn run<'a>(caller: &'a str, now_ms: i64) -> Run<'a> {
        Run { caller, now_ms, entropy: "00112233445566778899aabbccddeeff" }
    }

    fn act(host: &FakeHost, action: &str, caller: &str, now_ms: i64) -> Value {
        let input = json!({"action": action, "state": "ignored"}).to_string();
        serde_json::from_str(&apply(host, "wordguess", &input, &run(caller, now_ms)).unwrap()).unwrap()
    }

    fn letters(scene: &Value) -> String {
        scene["ops"].as_array().unwrap().iter().filter_map(|o| o["s"].as_str()).collect::<Vec<_>>().join("")
    }

    fn wrong_word() -> &'static str {
        ["slate", "crane", "pound", "fight"].into_iter().find(|w| *w != answer_for(DAY)).unwrap()
    }

    fn solve(host: &FakeHost, caller: &str, now_ms: i64) {
        let word = answer_for(day_of(now_ms));
        act(host, &format!("guess:{word}"), caller, now_ms);
    }

    #[test]
    fn launching_shows_the_shared_landing_and_unknown_actions_return_to_it() {
        let host = FakeHost::default();
        let launched: Value = serde_json::from_str(&apply(&host, "wordguess", "", &run("ann", NOON)).unwrap()).unwrap();
        assert!(launched.get("private").is_none());
        assert_eq!(launched["controls"], json!(["play today", "practice"]));
        assert_eq!(host.calls.get(), 0, "the landing needs nothing from the host");
        for action in ["close", "nonsense", ""] {
            assert!(act(&host, action, "ann", NOON).get("private").is_none(), "{action}");
        }
        assert!(apply(&host, "nope", "", &run("ann", NOON)).is_err());
    }

    #[test]
    fn two_players_the_same_day_get_the_same_word_on_separate_boards() {
        let host = FakeHost::default();
        let ann = act(&host, &format!("guess:{}", wrong_word()), "ann", NOON);
        let bob = act(&host, "play today", "bob", NOON + 1_000);
        assert_eq!(ann["private"], true);
        assert_eq!(letters(&ann), wrong_word().to_uppercase());
        assert_eq!(letters(&bob), "", "one finishing does not show their letters to the other");
        let bob_guess = act(&host, &format!("guess:{}", answer_for(DAY)), "bob", NOON + 2_000);
        assert!(bob_guess["status"].as_str().unwrap().starts_with("got it in 1"));
        let ann_again = act(&host, "play today", "ann", NOON + 3_000);
        assert_eq!(letters(&ann_again), wrong_word().to_uppercase(), "ann's board is still her own");
    }

    #[test]
    fn a_board_survives_between_calls_and_the_clients_own_state_is_ignored() {
        let host = FakeHost::default();
        act(&host, &format!("guess:{}", wrong_word()), "ann", NOON);
        let forged = format!("{DAY}|{},{},{}", answer_for(DAY), answer_for(DAY), answer_for(DAY));
        let input = json!({"action": "play today", "state": forged}).to_string();
        let again: Value = serde_json::from_str(&apply(&host, "wordguess", &input, &run("ann", NOON + 5_000)).unwrap()).unwrap();
        assert_eq!(letters(&again), wrong_word().to_uppercase());
        assert!(again["status"].as_str().unwrap().starts_with("1 of 6"), "{}", again["status"]);
    }

    #[test]
    fn a_finished_day_shows_the_finished_board_and_takes_no_more_guesses() {
        let host = FakeHost::default();
        solve(&host, "ann", NOON);
        let shown = act(&host, "play today", "ann", NOON + 10_000);
        assert!(shown["status"].as_str().unwrap().starts_with("got it in 1"));
        assert!(!shown["ops"].as_array().unwrap().iter().any(|o| o["op"] == "input"), "no input once finished");
        assert_eq!(shown["controls"], json!(["share", "practice", "close"]));
        let stored = host.plays.borrow().clone();
        let more = act(&host, &format!("guess:{}", wrong_word()), "ann", NOON + 20_000);
        assert_eq!(letters(&more), answer_for(DAY).to_uppercase());
        assert_eq!(*host.plays.borrow(), stored, "a second guess the same day changed nothing");
    }

    #[test]
    fn running_out_of_tries_finishes_the_day_and_reveals_the_word_to_that_player_only() {
        let host = FakeHost::default();
        let mut last = Value::Null;
        for word in ["aaaaa", "bbbbb", "ddddd", "eeeee", "fffff", "ggggg"] {
            last = act(&host, &format!("guess:{word}"), "ann", NOON);
        }
        assert!(last["status"].as_str().unwrap().contains(&answer_for(DAY).to_uppercase()), "{}", last["status"]);
        let bob = act(&host, "play today", "bob", NOON);
        assert!(!bob["status"].as_str().unwrap().contains(&answer_for(DAY).to_uppercase()));
    }

    #[test]
    fn tomorrow_is_a_new_word_and_a_new_board() {
        let host = FakeHost::default();
        solve(&host, "ann", NOON);
        let next = act(&host, "play today", "ann", NOON + DAY_MS);
        assert_eq!(letters(&next), "");
        assert!(next["status"].as_str().unwrap().starts_with("0 of 6"));
        assert_ne!(answer_for(DAY), answer_for(DAY + 1));
    }

    #[test]
    fn a_guess_that_cannot_be_used_says_why_and_is_not_stored() {
        let host = FakeHost::default();
        let short = act(&host, "guess:abc", "ann", NOON);
        assert_eq!(short["status"], "5 letters, a to z");
        act(&host, &format!("guess:{}", wrong_word()), "ann", NOON);
        let repeat = act(&host, &format!("guess:{}", wrong_word()), "ann", NOON);
        assert_eq!(repeat["status"], "already guessed");
        let stored = host.plays.borrow();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0]["guesses"], wrong_word());
    }

    #[test]
    fn sharing_waits_for_a_finished_board_and_posts_tries_and_tiles_without_the_word() {
        let host = FakeHost::default();
        act(&host, &format!("guess:{}", wrong_word()), "ann", NOON);
        let early = act(&host, "share", "ann", NOON);
        assert_eq!(early["status"], "finish today's word before sharing it");
        assert!(host.posts.borrow().is_empty());

        solve(&host, "ann", NOON);
        let shared = act(&host, "share", "ann", NOON);
        assert_eq!(shared["status"], "posted to the channel");
        let posts = host.posts.borrow();
        assert_eq!(posts.len(), 1);
        assert!(posts[0].starts_with(&format!("Word Guess day {DAY} 2/6")), "{}", posts[0]);
        assert!(!posts[0].to_lowercase().contains(answer_for(DAY)));
        assert!(posts[0].contains("#####"));
    }

    #[test]
    fn a_post_the_host_refuses_is_said_not_swallowed() {
        let host = FakeHost::default();
        solve(&host, "ann", NOON);
        host.refuse_post.set(true);
        let shared = act(&host, "share", "ann", NOON);
        assert!(shared["status"].as_str().unwrap().contains("approve"), "{}", shared["status"]);
    }

    #[test]
    fn practice_is_a_private_random_board_that_keeps_its_state_in_storage() {
        let host = FakeHost::default();
        let start = act(&host, "practice", "ann", NOON);
        assert_eq!(start["private"], true);
        assert_eq!(start["controls"], json!(["new practice", "today", "close"]));
        let one = act(&host, "pguess:crane", "ann", NOON);
        assert_eq!(letters(&one), "CRANE");
        let two = act(&host, "pguess:slate", "ann", NOON);
        assert_eq!(letters(&two), "CRANESLATE");
        let other = act(&host, "pguess:crane", "bob", NOON);
        assert_eq!(letters(&other), "CRANE", "bob's practice is his own, started on the spot");
        let streak_rows = host.plays.borrow().iter().filter(|r| r["day"] == -1).count();
        assert_eq!(streak_rows, 2, "one practice row per player, no daily row");
    }

    #[test]
    fn practice_boards_differ_between_launches_and_between_players() {
        let host = FakeHost::default();
        let seed_of = |caller: &str, entropy: &str| {
            let input = json!({"action": "practice"}).to_string();
            apply(&host, "wordguess", &input, &Run { caller, now_ms: NOON, entropy }).unwrap();
            Plays::new(&host).load_practice(caller).unwrap().seed
        };
        let a = seed_of("ann", "00112233445566778899aabbccddeeff");
        let b = seed_of("ann", "ffeeddccbbaa99887766554433221100");
        let c = seed_of("bob", "00112233445566778899aabbccddeeff");
        assert!(a != b && a != c && a != 0);
    }

    #[test]
    fn a_streak_shows_on_the_board_after_consecutive_wins() {
        let host = FakeHost::default();
        for day in 0..3 {
            solve(&host, "ann", NOON - day * DAY_MS);
        }
        let board = act(&host, "play today", "ann", NOON);
        assert!(board["status"].as_str().unwrap().contains("3 days in a row"), "{}", board["status"]);
    }

    #[test]
    fn a_host_that_has_not_approved_the_store_gets_a_sentence_not_a_blank_board() {
        let host = FakeHost::default();
        host.refuse.set(true);
        let board = act(&host, "play today", "ann", NOON);
        assert!(board["status"].as_str().unwrap().contains("An admin needs to approve"), "{}", board["status"]);
        assert_eq!(board["private"], true);
    }

    #[test]
    fn a_caller_id_cannot_break_the_key_it_is_stored_under() {
        let host = FakeHost::default();
        act(&host, &format!("guess:{}", wrong_word()), "a|b\"c\u{0}", NOON);
        let key = host.plays.borrow()[0]["k"].as_str().unwrap().to_owned();
        assert_eq!(key, format!("{DAY}|abc"));
    }
}
