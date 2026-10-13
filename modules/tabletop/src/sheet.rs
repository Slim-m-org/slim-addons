//! One short character sheet per player, kept in `kv.store` under the caller's
//! opaque id. A sheet stays under 256 bytes, so 256 players fit in the 64 KiB
//! a module may hold.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::kv::Kv;

pub const HELP: &str = "Sheet commands:\n\
/sheet - show yours\n\
/sheet name Aria\n\
/sheet set str 16 (or /sheet str 16)\n\
/sheet del str\n\
/sheet hp 12/20 - set hp and max, /sheet hp 9 - set hp, /sheet hp -3 or +5 - change hp\n\
/sheet clear - delete your sheet";

const MAX_NAME_CHARS: usize = 24;
const MAX_STATS: usize = 8;
const MAX_STAT_CHARS: usize = 8;
const MAX_SHEET_BYTES: usize = 256;
const MAX_CALLER_CHARS: usize = 64;

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Sheet {
    #[serde(rename = "n", default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hp: Option<i32>,
    #[serde(rename = "mx", default, skip_serializing_if = "Option::is_none")]
    pub max_hp: Option<i32>,
    #[serde(rename = "s", default)]
    pub stats: BTreeMap<String, i32>,
}

fn key(caller: &str) -> String {
    format!("sheet:{caller}")
}

fn load(kv: &dyn Kv, key: &str) -> Result<Option<Sheet>, String> {
    let raw = kv.get(key).map_err(unavailable)?;
    Ok(raw.map(|text| serde_json::from_str(&text).unwrap_or_default()))
}

fn unavailable(reason: String) -> String {
    format!("sheets need the kv.store capability, which an admin approves for this module ({reason})")
}

pub fn show(sheet: &Sheet) -> String {
    let mut lines = vec![if sheet.name.is_empty() { "(unnamed)".to_string() } else { sheet.name.clone() }];
    match (sheet.hp, sheet.max_hp) {
        (Some(hp), Some(max)) => lines.push(format!("HP {hp}/{max}")),
        (Some(hp), None) => lines.push(format!("HP {hp}")),
        _ => {}
    }
    if !sheet.stats.is_empty() {
        let stats: Vec<String> = sheet.stats.iter().map(|(k, v)| format!("{k} {v}")).collect();
        lines.push(stats.join(", "));
    }
    lines.join("\n")
}

fn clean_stat(word: &str) -> Result<String, String> {
    let stat = word.to_ascii_lowercase();
    let valid = !stat.is_empty() && stat.chars().count() <= MAX_STAT_CHARS && stat.chars().all(|c| c.is_ascii_alphanumeric());
    if valid {
        Ok(stat)
    } else {
        Err(format!("a stat name is 1 to {MAX_STAT_CHARS} letters or digits, like str"))
    }
}

fn stat_value(word: &str) -> Result<i32, String> {
    word.parse::<i32>()
        .ok()
        .filter(|n| (-999..=9999).contains(n))
        .ok_or_else(|| "a stat value is a whole number from -999 to 9999".to_string())
}

fn set_hp(sheet: &mut Sheet, arg: &str) -> Result<(), String> {
    let bad = || "hp is a number like 9, a pair like 12/20, or a change like -3 or +5".to_string();
    let number = |text: &str| text.parse::<i32>().ok().filter(|n| (0..=9999).contains(n)).ok_or_else(bad);
    if arg.starts_with(['+', '-']) {
        let change: i32 = arg.parse().map_err(|_| bad())?;
        let hp = sheet.hp.ok_or("set your hp first, like /sheet hp 12/20")?;
        sheet.hp = Some((hp + change).clamp(0, sheet.max_hp.unwrap_or(9999)));
        return Ok(());
    }
    if let Some((hp, max)) = arg.split_once('/') {
        let (hp, max) = (number(hp)?, number(max)?);
        sheet.max_hp = Some(max.max(1));
        sheet.hp = Some(hp.min(max.max(1)));
        return Ok(());
    }
    sheet.hp = Some(number(arg)?.min(sheet.max_hp.unwrap_or(9999)));
    Ok(())
}

fn edit(sheet: &mut Sheet, verb: &str, rest: &[&str]) -> Result<(), String> {
    match (verb, rest) {
        ("name", words) if !words.is_empty() => {
            sheet.name = words.join(" ").chars().filter(|c| !c.is_control()).take(MAX_NAME_CHARS).collect();
        }
        ("set", [stat, value]) => set_stat(sheet, stat, value)?,
        ("del", [stat]) => {
            let stat = clean_stat(stat)?;
            sheet.stats.remove(&stat).ok_or_else(|| format!("no stat called {stat}"))?;
        }
        ("hp", [arg]) => set_hp(sheet, arg)?,
        (stat, [value]) if stat.chars().all(|c| c.is_ascii_alphanumeric()) => set_stat(sheet, stat, value)?,
        _ => return Err(format!("not sure what that means\n{HELP}")),
    }
    Ok(())
}

fn set_stat(sheet: &mut Sheet, stat: &str, value: &str) -> Result<(), String> {
    let stat = clean_stat(stat)?;
    let value = stat_value(value)?;
    if !sheet.stats.contains_key(&stat) && sheet.stats.len() >= MAX_STATS {
        return Err(format!("a sheet holds at most {MAX_STATS} stats; /sheet del one first"));
    }
    sheet.stats.insert(stat, value);
    Ok(())
}

/// The `sheet` command for `caller`: show their sheet, or apply one edit to it and show the result.
pub fn run(kv: &dyn Kv, caller: &str, input: &str) -> Result<String, String> {
    let words: Vec<&str> = input.split_whitespace().collect();
    let verb = words.first().map(|w| w.to_ascii_lowercase()).unwrap_or_default();
    if verb == "help" {
        return Ok(HELP.to_string());
    }
    if caller.is_empty() || caller.chars().count() > MAX_CALLER_CHARS {
        return Err("slim did not say who is asking, so there is no sheet to keep".to_string());
    }
    let key = key(caller);
    let stored = load(kv, &key)?;
    match verb.as_str() {
        "" | "show" => Ok(stored.map_or_else(|| format!("no sheet yet\n{HELP}"), |sheet| show(&sheet))),
        "clear" => {
            kv.delete(&key).map_err(unavailable)?;
            Ok("sheet deleted".to_string())
        }
        _ => {
            let mut sheet = stored.unwrap_or_default();
            edit(&mut sheet, &verb, &words[1..])?;
            let value = serde_json::to_string(&sheet).map_err(|_| "could not save the sheet".to_string())?;
            if value.len() > MAX_SHEET_BYTES {
                return Err("that sheet is too long to keep; shorten the name or drop a stat".to_string());
            }
            kv.set(&key, &value).map_err(unavailable)?;
            Ok(show(&sheet))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kv::MemKv;

    const ALICE: &str = "a1ice";
    const BOB: &str = "b0b";

    fn sheet_of(kv: &MemKv, who: &str) -> Sheet {
        serde_json::from_str(&kv.map.borrow()[&key(who)]).unwrap()
    }

    #[test]
    fn building_a_sheet_persists_it_under_the_callers_key() {
        let kv = MemKv::default();
        run(&kv, ALICE, "name Aria Vale").unwrap();
        run(&kv, ALICE, "set str 16").unwrap();
        run(&kv, ALICE, "DEX 14").unwrap();
        let out = run(&kv, ALICE, "hp 12/20").unwrap();
        assert_eq!(out, "Aria Vale\nHP 12/20\ndex 14, str 16");
        assert_eq!(run(&kv, ALICE, "").unwrap(), out);
        assert_eq!(sheet_of(&kv, ALICE).stats["str"], 16);
    }

    #[test]
    fn sheets_belong_to_one_caller_each() {
        let kv = MemKv::default();
        run(&kv, ALICE, "name Aria").unwrap();
        assert!(run(&kv, BOB, "show").unwrap().starts_with("no sheet yet"));
        run(&kv, BOB, "name Bo").unwrap();
        assert_eq!(run(&kv, ALICE, "show").unwrap(), "Aria");
    }

    #[test]
    fn hp_can_be_set_changed_and_never_leaves_zero_to_max() {
        let kv = MemKv::default();
        assert!(run(&kv, ALICE, "hp -3").unwrap_err().contains("set your hp first"));
        run(&kv, ALICE, "hp 10/20").unwrap();
        assert!(run(&kv, ALICE, "hp -4").unwrap().contains("HP 6/20"));
        assert!(run(&kv, ALICE, "hp +99").unwrap().contains("HP 20/20"));
        assert!(run(&kv, ALICE, "hp -99").unwrap().contains("HP 0/20"));
        assert!(run(&kv, ALICE, "hp 50").unwrap().contains("HP 20/20"));
        assert!(run(&kv, ALICE, "hp 30/10").unwrap().contains("HP 10/10"));
        assert!(run(&kv, ALICE, "hp lots").unwrap_err().contains("hp is a number"));
    }

    #[test]
    fn stats_are_validated_capped_and_deletable() {
        let kv = MemKv::default();
        for i in 0..MAX_STATS {
            run(&kv, ALICE, &format!("set s{i} {i}")).unwrap();
        }
        assert!(run(&kv, ALICE, "set extra 1").unwrap_err().contains("at most 8"));
        run(&kv, ALICE, "set s0 99").unwrap();
        assert!(run(&kv, ALICE, "del s0").is_ok());
        assert!(run(&kv, ALICE, "del s0").unwrap_err().contains("no stat called s0"));
        assert!(run(&kv, ALICE, "set toolongstat 1").unwrap_err().contains("1 to 8"));
        assert!(run(&kv, ALICE, "set str many").unwrap_err().contains("whole number"));
        assert!(run(&kv, ALICE, "set str 10000").is_err());
    }

    #[test]
    fn clear_deletes_the_sheet_and_help_needs_no_store() {
        let kv = MemKv::default();
        run(&kv, ALICE, "name Aria").unwrap();
        assert_eq!(run(&kv, ALICE, "clear").unwrap(), "sheet deleted");
        assert!(kv.map.borrow().is_empty());
        kv.down.set(true);
        assert_eq!(run(&kv, "", "help").unwrap(), HELP);
        assert!(run(&kv, "", "show").unwrap_err().contains("who is asking"));
    }

    #[test]
    fn a_store_the_admin_has_not_approved_is_explained() {
        let kv = MemKv::default();
        kv.down.set(true);
        let err = run(&kv, ALICE, "name Aria").unwrap_err();
        assert!(err.contains("an admin approves"), "{err}");
    }

    #[test]
    fn an_edit_costs_two_store_calls_and_a_show_one() {
        let kv = MemKv::default();
        run(&kv, ALICE, "name Aria").unwrap();
        assert_eq!(kv.calls.get(), 2);
        run(&kv, ALICE, "show").unwrap();
        assert_eq!(kv.calls.get(), 3);
    }

    #[test]
    fn the_largest_sheet_is_small_enough_for_256_players_in_64_kib() {
        let kv = MemKv::default();
        let caller = "f".repeat(32);
        run(&kv, &caller, &format!("name {}", "W".repeat(40))).unwrap();
        for i in 0..MAX_STATS {
            run(&kv, &caller, &format!("set stat{i}xyz 9999")).unwrap();
        }
        run(&kv, &caller, "hp 9999/9999").unwrap();
        let map = kv.map.borrow();
        let (k, v) = map.iter().next().unwrap();
        assert!(v.len() <= MAX_SHEET_BYTES, "{} bytes", v.len());
        assert!(256 * (k.len() + v.len()) <= 64 * 1024, "{} bytes", 256 * (k.len() + v.len()));
    }

    #[test]
    fn nonsense_gets_the_help() {
        let kv = MemKv::default();
        assert!(run(&kv, ALICE, "frobnicate the sheet").unwrap_err().contains("Sheet commands"));
        assert!(run(&kv, ALICE, "name").unwrap_err().contains("Sheet commands"));
    }
}
