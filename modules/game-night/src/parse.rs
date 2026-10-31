//! Reads `game | fri 8pm, sat 9pm | role` into a plan: the game, up to six start times, and an optional role to ping.

const DAY_MS: i64 = 86_400_000;
const MAX_SLOTS: usize = 6;
const MAX_GAME: usize = 40;
const MAX_ROLE: usize = 32;
const MAX_AHEAD_DAYS: i64 = 60;
const WEEKDAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
pub const USAGE: &str = "type: game | fri 8pm, sat 9pm | role to ping (optional)";

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Slot {
    pub label: String,
    pub start_ms: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub game: String,
    pub slots: Vec<Slot>,
    pub role: String,
}

/// `offset_min` is how far the typed times sit ahead of UTC.
pub fn plan(text: &str, now_ms: i64, offset_min: i64) -> Result<Plan, String> {
    let parts: Vec<&str> = text.split('|').map(str::trim).collect();
    let (game, times, role) = match parts.as_slice() {
        [game, times] => (*game, *times, ""),
        [game, times, role] => (*game, *times, *role),
        _ => return Err(USAGE.to_owned()),
    };
    let game = clean(game, MAX_GAME);
    if game.is_empty() {
        return Err(USAGE.to_owned());
    }
    let mut slots: Vec<Slot> = Vec::new();
    for spec in times.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let slot = slot(spec, now_ms, offset_min)?;
        if !slots.iter().any(|s| s.start_ms == slot.start_ms) {
            slots.push(slot);
        }
    }
    if slots.is_empty() || slots.len() > MAX_SLOTS {
        return Err(format!("give 1 to {MAX_SLOTS} times, like fri 8pm"));
    }
    let role = clean(&role.replace(['@', '[', ']'], ""), MAX_ROLE);
    Ok(Plan { game, slots, role })
}

fn clean(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(max).collect()
}

fn slot(spec: &str, now_ms: i64, offset_min: i64) -> Result<Slot, String> {
    let lowered = spec.to_lowercase();
    let mut day_word = None;
    let mut minutes = None;
    let mut words: Vec<String> = lowered.split_whitespace().map(str::to_owned).collect();
    // "8 pm" is two words; glue the meridiem back on.
    let mut i = 0;
    while i + 1 < words.len() {
        if matches!(words[i + 1].as_str(), "am" | "pm") && words[i].chars().all(|c| c.is_ascii_digit() || c == ':') {
            let meridiem = words.remove(i + 1);
            words[i].push_str(&meridiem);
        }
        i += 1;
    }
    for word in &words {
        if let Some(m) = time_of_day(word) {
            minutes = Some(m);
        } else {
            day_word = Some(word.as_str());
        }
    }
    let (Some(day_word), Some(minutes)) = (day_word, minutes) else {
        return Err(format!("{spec} needs a day and a time, like fri 8pm"));
    };
    let local_now = now_ms + offset_min * 60_000;
    let today = local_now.div_euclid(DAY_MS);
    let (mut day, recurring) = day_of(day_word, today).ok_or_else(|| format!("{day_word} is not a day I know: use today, tomorrow, a weekday or YYYY-MM-DD"))?;
    let start = |day: i64| day * DAY_MS + minutes * 60_000 - offset_min * 60_000;
    if start(day) <= now_ms && recurring {
        day += 7;
    }
    let start_ms = start(day);
    if start_ms <= now_ms {
        return Err(format!("{spec} has already passed"));
    }
    if day - today > MAX_AHEAD_DAYS {
        return Err(format!("{spec} is more than {MAX_AHEAD_DAYS} days away"));
    }
    Ok(Slot { label: label(day, minutes, day - today), start_ms })
}

/// The day number a word names, and whether a past time today should roll to next week.
fn day_of(word: &str, today: i64) -> Option<(i64, bool)> {
    match word {
        "today" => return Some((today, false)),
        "tomorrow" => return Some((today + 1, false)),
        _ => {}
    }
    if let Some(day) = iso(word) {
        return Some((day, false));
    }
    let index = WEEKDAYS.iter().position(|d| word.len() >= 3 && word.starts_with(d))?;
    let today_weekday = (today + 4).rem_euclid(7);
    Some((today + (index as i64 - today_weekday).rem_euclid(7), true))
}

fn iso(word: &str) -> Option<i64> {
    let mut parts = word.split('-');
    let (y, m, d) = (parts.next()?.parse::<i64>().ok()?, parts.next()?.parse::<i64>().ok()?, parts.next()?.parse::<i64>().ok()?);
    if parts.next().is_some() || !(1970..=2200).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let days = days_from_civil(y, m, d);
    (civil_from_days(days) == (y, m, d)).then_some(days)
}

fn time_of_day(word: &str) -> Option<i64> {
    let (core, meridiem) = match word.strip_suffix("pm") {
        Some(core) => (core, Some(true)),
        None => match word.strip_suffix("am") {
            Some(core) => (core, Some(false)),
            None => (word, None),
        },
    };
    let (hour, minute) = match core.split_once(':') {
        Some((h, m)) => (h.parse::<i64>().ok()?, m.parse::<i64>().ok()?),
        None if meridiem.is_some() => (core.parse::<i64>().ok()?, 0),
        None => return None,
    };
    if !(0..60).contains(&minute) || core.starts_with(':') {
        return None;
    }
    let hour = match meridiem {
        Some(pm) if (1..=12).contains(&hour) => hour % 12 + if pm { 12 } else { 0 },
        Some(_) => return None,
        None if (0..24).contains(&hour) => hour,
        None => return None,
    };
    Some(hour * 60 + minute)
}

fn label(day: i64, minutes: i64, away: i64) -> String {
    let weekday = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][(day + 4).rem_euclid(7) as usize];
    let clock = format!("{:02}:{:02}", minutes / 60, minutes % 60);
    if away < 7 {
        return format!("{weekday} {clock}");
    }
    let (_, month, date) = civil_from_days(day);
    format!("{weekday} {date} {} {clock}", MONTHS[(month - 1) as usize])
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tuesday 2023-11-14 22:13:20 UTC.
    const NOW: i64 = 1_700_000_000_000;

    fn labels(text: &str, offset: i64) -> Vec<String> {
        plan(text, NOW, offset).unwrap().slots.into_iter().map(|s| s.label).collect()
    }

    #[test]
    fn a_plan_reads_the_game_the_times_and_the_role() {
        let p = plan("Valorant | fri 8pm, sat 21:30 | Gamers", NOW, 0).unwrap();
        assert_eq!((p.game.as_str(), p.role.as_str()), ("Valorant", "Gamers"));
        assert_eq!(p.slots[0].label, "Fri 20:00");
        assert_eq!(p.slots[0].start_ms, NOW - (22 * 3600 + 13 * 60 + 20) * 1000 + (3 * 24 + 20) * 3_600_000);
        assert_eq!(p.slots[1].label, "Sat 21:30");
        assert_eq!(plan("Chess | tomorrow 7 pm", NOW, 0).unwrap().role, "");
    }

    #[test]
    fn times_are_read_in_the_offset_and_a_passed_weekday_rolls_to_next_week() {
        // Tuesday 22:13 UTC is already Wednesday 00:13 at +120, so Tuesday evening is a week off.
        assert_eq!(labels("g | tue 8pm", 120), ["Tue 20:00"]);
        assert_eq!(labels("g | tue 8pm", 0), ["Tue 21 Nov 20:00"]);
        assert_eq!(labels("g | tuesday 11pm", 0), ["Tue 23:00"]);
        assert_eq!(labels("g | wed 8pm", 120), ["Wed 20:00"]);
    }

    #[test]
    fn dates_far_off_carry_their_day_and_month() {
        assert_eq!(labels("g | 2023-12-01 18:00", 0), ["Fri 1 Dec 18:00"]);
        assert!(plan("g | 2024-06-01 18:00", NOW, 0).unwrap_err().contains("60 days"));
        assert!(plan("g | 2023-11-14 08:00", NOW, 0).unwrap_err().contains("passed"));
    }

    #[test]
    fn nonsense_is_refused_with_a_hint() {
        for bad in ["", "game", "game | ", "game | fri", "game | 8pm", "game | blursday 8pm", "game | fri 25:00", "game | fri 13pm", "game | 2023-02-30 10:00", " | fri 8pm", "a | b | c | d"] {
            assert!(plan(bad, NOW, 0).is_err(), "{bad:?}");
        }
        assert!(plan("g | fri 8pm, sat 8pm, sun 8pm, mon 8pm, tue 8pm, wed 8pm, thu 9pm", NOW, 0).is_err());
    }

    #[test]
    fn duplicates_collapse_and_the_role_loses_mention_syntax() {
        assert_eq!(plan("g | fri 8pm, friday 20:00", NOW, 0).unwrap().slots.len(), 1);
        assert_eq!(plan("g | fri 8pm | @[Core Team]\nx", NOW, 0).unwrap().role, "Core Teamx");
    }
}
