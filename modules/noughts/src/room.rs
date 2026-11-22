//! The `room/1` contract: the host's input in, one of `state`, `reject` or
//! `end` out. The host keeps the state between inputs and runs one at a time.

use serde::Deserialize;
use serde_json::json;

use crate::game::{self, Game, Step};

pub const COMMAND: &str = "@room";
const CONTRACT: &str = "room/1";

#[derive(Deserialize)]
struct Input {
    #[serde(rename = "$slim")]
    slim: String,
    #[serde(default)]
    state: Option<Game>,
    input: String,
}

/// The module's answer to one room input, as the `output` text.
pub fn answer(input: &str, caller: &str) -> String {
    let parsed = match serde_json::from_str::<Input>(input) {
        Ok(parsed) if parsed.slim == CONTRACT => parsed,
        _ => return reject("that is not a room/1 input"),
    };
    let game = parsed.state.map(Game::checked).unwrap_or_default();
    match game::apply(game, parsed.input.trim(), caller) {
        Ok(Step::Next(next)) => json!({ "$slim": CONTRACT, "state": next }).to_string(),
        Ok(Step::End) => json!({ "$slim": CONTRACT, "end": true }).to_string(),
        Err(why) => reject(why),
    }
}

fn reject(why: &str) -> String {
    json!({ "$slim": CONTRACT, "reject": why }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn reply(input: &str, caller: &str) -> Value {
        serde_json::from_str(&answer(input, caller)).unwrap()
    }

    #[test]
    fn a_first_move_opens_the_board() {
        let input = r#"{"$slim":"room/1","room":{"channel_id":"c","kind":"voice"},"seq":0,"state":null,"input":"4"}"#;
        let out = reply(input, "aa");
        assert_eq!(out["$slim"], "room/1");
        assert_eq!(out["state"]["board"], "....x....");
        assert_eq!(out["state"]["x"], "aa");
    }

    #[test]
    fn the_documented_input_fixture_gets_the_documented_state() {
        let input = r#"{"$slim":"room/1","room":{"channel_id":"01a1a929-1827-7e11-b45f-fa8008c5ac06","kind":"voice"},"seq":1,"state":{"board":"x........","next":"o","x":"5f2c7d0e9a1b3c4d","o":null,"winner":null},"input":"4"}"#;
        let out = reply(input, "a8e1f6b2c9d04e7f");
        assert_eq!(out["state"]["board"], "x...o....");
        assert_eq!(out["state"]["o"], "a8e1f6b2c9d04e7f");
        assert_eq!(out["state"]["next"], "x");
    }

    #[test]
    fn a_refused_move_rejects_and_end_ends() {
        let state = r#"{"board":"x........","next":"o","x":"aa","o":null,"winner":null}"#;
        let input = |sent: &str| {
            format!(r#"{{"$slim":"room/1","room":{{"channel_id":"c","kind":"voice"}},"seq":1,"state":{state},"input":"{sent}"}}"#)
        };
        assert_eq!(reply(&input("1"), "aa")["reject"], "not your turn");
        assert_eq!(reply(&input("end"), "aa")["end"], true);
    }

    #[test]
    fn anything_that_is_not_room_1_is_rejected() {
        assert_eq!(reply("nope", "aa")["reject"], "that is not a room/1 input");
        assert_eq!(
            reply(r#"{"$slim":"room/2","input":"1"}"#, "aa")["reject"],
            "that is not a room/1 input"
        );
    }
}
