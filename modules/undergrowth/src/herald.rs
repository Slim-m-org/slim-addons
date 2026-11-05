//! The two things the module says on its own, as itself, in the channel the admin picked:
//! a finished season's results (on a timer the season's last frame sets) and a daily almanac.

use serde_json::Value;

use crate::events::catch_up_calls;
use crate::host::Host;
use crate::settings::Settings;
use crate::store::Store;
use crate::words::count;

const DAY_MS: i64 = 86_400_000;

/// Posts a finished season's results once, however many times it is asked.
pub fn herald<H: Host>(store: &Store<H>, game: &str) -> Result<String, String> {
    let game = if game.trim().is_empty() {
        match store.latest_game()? {
            Some(row) => row["game"].as_str().unwrap_or("").to_owned(),
            None => return Ok("no season has been played yet".into()),
        }
    } else {
        game.trim().to_owned()
    };
    let Some(row) = store.claim_posting(&game)? else { return Ok("nothing new to announce".into()) };
    let text = results(&row);
    store.post(&text)?;
    Ok(text)
}

fn results(row: &Value) -> String {
    let standings: Value = serde_json::from_str(row["standings"].as_str().unwrap_or("{}")).unwrap_or(Value::Null);
    let season = row["ended"].as_i64().unwrap_or(0);
    let colonies = standings["colonies"].as_array().cloned().unwrap_or_default();
    let leaves = standings["leaves"].as_i64().unwrap_or(0);
    let Some(first) = colonies.first() else {
        return format!("Season {season} of the undergrowth ended with no colonies in it.");
    };
    let n = |v: &Value, k: &str| v[k].as_i64().unwrap_or(0);
    let mut text = format!(
        "Season {season} of the undergrowth is over. {} won with {} points: {} cells at its peak and {} standing.",
        first["name"].as_str().unwrap_or("a colony"),
        n(first, "total"),
        n(first, "peak"),
        count(n(first, "mushrooms"), "mushroom", "mushrooms"),
    );
    let rest: Vec<String> =
        colonies.iter().skip(1).map(|c| format!("{} {}", c["name"].as_str().unwrap_or("a colony"), n(c, "total"))).collect();
    if !rest.is_empty() {
        text.push_str(&format!(" Then {}.", rest.join(", ")));
    }
    if leaves > 0 {
        text.push_str(&format!(" {} fell from your messages along the way.", count(leaves, "leaf", "leaves")));
    }
    text
}

/// Once a day: count what the space fed the soil, post it, and tidy rows nothing needs any more.
pub fn almanac<H: Host>(store: &Store<H>, settings: &Settings, now_ms: i64) -> Result<String, String> {
    let calls = catch_up_calls(store, now_ms).unwrap_or(0);
    let since = now_ms - DAY_MS;
    let leaves = store.count_since("leaf", since)?;
    let rain = store.count_since("rain", since)?;
    let warm = store.sum_since("warm", since)?;
    let latest = store.latest_game()?.filter(|g| g["updated_ms"].as_i64().unwrap_or(0) > since && has_colonies(g));
    store.prune("inbox", now_ms - 3 * DAY_MS)?;
    store.prune("moves", now_ms - 2 * DAY_MS)?;
    store.prune("links", now_ms - DAY_MS)?;
    if !settings.almanac {
        return Ok(format!("the almanac is switched off; tidied up and counted {calls} calls"));
    }
    if leaves + rain + warm == 0 && latest.is_none() {
        return Ok("a quiet day; nothing to post".into());
    }
    let day = now_ms / DAY_MS;
    let last = store.fact("almanac_day")?.and_then(|r| r["num"].as_i64()).unwrap_or(0);
    if last >= day || !store.advance_fact("almanac_day", last, day)? {
        return Ok("today's almanac is already out".into());
    }
    let text = digest((leaves, rain, warm), latest.as_ref());
    store.post(&text)?;
    Ok(text)
}

fn has_colonies(game: &Value) -> bool {
    let standings: Value = serde_json::from_str(game["standings"].as_str().unwrap_or("{}")).unwrap_or(Value::Null);
    standings["colonies"].as_array().is_some_and(|c| !c.is_empty())
}

pub fn digest((leaves, rain, warm): (i64, i64, i64), latest: Option<&Value>) -> String {
    let mut fed = Vec::new();
    if leaves > 0 {
        fed.push(format!("{} fell from your messages", count(leaves, "leaf", "leaves")));
    }
    if rain > 0 {
        fed.push(format!("{} came with reactions", count(rain, "shower", "showers")));
    }
    if warm > 0 {
        fed.push(format!("calls kept it warm for {} h {} min", warm / 60, warm % 60));
    }
    let mut text = String::from("The undergrowth, last 24 hours:");
    match fed.len() {
        0 => text.push_str(" quiet soil."),
        1 => text.push_str(&format!(" {}.", fed[0])),
        n => text.push_str(&format!(" {}, and {}.", fed[..n - 1].join(", "), fed[n - 1])),
    }
    if let Some(game) = latest {
        let standings: Value = serde_json::from_str(game["standings"].as_str().unwrap_or("{}")).unwrap_or(Value::Null);
        let tops: Vec<String> = standings["colonies"]
            .as_array()
            .into_iter()
            .flatten()
            .take(3)
            .map(|c| format!("{} {} cells", c["name"].as_str().unwrap_or("a colony"), c["cells"].as_i64().unwrap_or(0)))
            .collect();
        if !tops.is_empty() {
            let state = if standings["over"].as_bool() == Some(true) { "ended" } else { "so far" };
            text.push_str(&format!(" Season {} {state}: {}.", standings["season"].as_i64().unwrap_or(0), tops.join(", ")));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn results_name_the_winner_and_the_rest() {
        let standings = json!({"leaves": 12, "colonies": [
            {"name": "Ink Cap", "total": 900, "peak": 700, "mushrooms": 5},
            {"name": "Ash Rot", "total": 300, "peak": 280, "mushrooms": 0}]});
        let row = json!({"ended": 3, "standings": standings.to_string()});
        let text = results(&row);
        assert!(text.starts_with("Season 3 of the undergrowth is over. Ink Cap won with 900 points"), "{text}");
        assert!(text.contains("Then Ash Rot 300.") && text.contains("12 leaves") && text.contains("5 mushrooms standing"), "{text}");
        assert!(results(&json!({"ended": 2})).contains("no colonies"));
    }

    #[test]
    fn a_digest_says_what_fed_the_soil() {
        let text = digest((10, 2, 95), None);
        assert_eq!(text, "The undergrowth, last 24 hours: 10 leaves fell from your messages, 2 showers came with reactions, and calls kept it warm for 1 h 35 min.");
        assert_eq!(digest((1, 1, 0), None), "The undergrowth, last 24 hours: 1 leaf fell from your messages, and 1 shower came with reactions.");
        let game = json!({"standings": json!({"season": 4, "over": false, "colonies": [{"name": "Moss", "cells": 80}]}).to_string()});
        assert!(digest((0, 0, 0), Some(&game)).ends_with("Season 4 so far: Moss 80 cells."));
    }
}
