//! Reading a start time the way a person types it, and showing it back in the zone they typed.
//!
//! Everything is stored as UTC milliseconds plus the zone offset the host typed, because the
//! run request has a clock but no time zone.

pub const USAGE: &str = "when: in 2h, or 2026-01-05 20:00 +02:00 (UTC without a zone), or tomorrow 8pm";

const MINUTE: i64 = 60_000;
const DAY: i64 = 86_400_000;
const MAX_AHEAD_DAYS: i64 = 366;
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
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

/// When `at_ms` is, in the zone `zone_min` minutes from UTC: "Mon 5 Jan 2026, 20:00 UTC+02:00".
pub fn format_local(at_ms: i64, zone_min: i32) -> String {
    let local = at_ms + i64::from(zone_min) * MINUTE;
    let days = local.div_euclid(DAY);
    let minutes = local.rem_euclid(DAY) / MINUTE;
    let (y, m, d) = civil_from_days(days);
    let weekday = WEEKDAYS[days.rem_euclid(7) as usize];
    format!("{weekday} {d} {} {y}, {:02}:{:02} {}", MONTHS[(m - 1) as usize], minutes / 60, minutes % 60, zone_name(zone_min))
}

pub fn zone_name(zone_min: i32) -> String {
    match zone_min {
        0 => "UTC".to_owned(),
        _ => format!("UTC{}{:02}:{:02}", if zone_min < 0 { '-' } else { '+' }, zone_min.abs() / 60, zone_min.abs() % 60),
    }
}

/// How far off a start is, or how long ago it was.
pub fn countdown(ms: i64) -> String {
    let (minutes, past) = (ms.abs() / MINUTE, ms < 0);
    let span = match minutes {
        0 => return "starting now".to_owned(),
        1..=59 => format!("{minutes}m"),
        60..=1439 => format!("{}h {}m", minutes / 60, minutes % 60),
        _ => format!("{}d {}h", minutes / 1440, minutes % 1440 / 60),
    };
    if past { format!("started {span} ago") } else { format!("starts in {span}") }
}

/// A start time as UTC milliseconds and the zone offset (minutes) it was typed in.
pub fn parse_when(text: &str, now_ms: i64) -> Result<(i64, i32), String> {
    let words: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
    let (at, zone) = match words.first().map(String::as_str) {
        None => return Err(USAGE.to_owned()),
        Some("in") => (now_ms + parse_span(&words[1..].concat())?, 0),
        Some(_) => parse_absolute(&words, now_ms)?,
    };
    if at <= now_ms {
        return Err("that time has already passed".to_owned());
    }
    if at - now_ms > MAX_AHEAD_DAYS * DAY {
        return Err("events can be at most a year ahead".to_owned());
    }
    Ok((at, zone))
}

/// `1h30m`, `2d`, `45min`: at least one minute.
fn parse_span(text: &str) -> Result<i64, String> {
    let (mut total, mut digits) = (0i64, String::new());
    let mut rest = text.chars().peekable();
    while let Some(c) = rest.next() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let mut unit = c.to_string();
        while let Some(&next) = rest.peek().filter(|n| n.is_ascii_alphabetic()) {
            unit.push(next);
            rest.next();
        }
        let per = match unit.as_str() {
            "m" | "min" | "mins" => MINUTE,
            "h" | "hr" | "hrs" | "hour" | "hours" => 60 * MINUTE,
            "d" | "day" | "days" => DAY,
            "w" | "week" | "weeks" => 7 * DAY,
            _ => return Err(USAGE.to_owned()),
        };
        let count: i64 = digits.parse().map_err(|_| USAGE.to_owned())?;
        total = total.saturating_add(count.saturating_mul(per));
        digits.clear();
    }
    if !digits.is_empty() || total < MINUTE {
        return Err(USAGE.to_owned());
    }
    Ok(total)
}

fn parse_absolute(words: &[String], now_ms: i64) -> Result<(i64, i32), String> {
    let zone = match words.get(2) {
        Some(word) => parse_zone(word)?,
        None => 0,
    };
    if words.len() > 3 || words.len() < 2 {
        return Err(USAGE.to_owned());
    }
    let today = (now_ms + i64::from(zone) * MINUTE).div_euclid(DAY);
    let day = match words[0].as_str() {
        "today" => today,
        "tomorrow" => today + 1,
        date => parse_date(date)?,
    };
    let minutes = parse_clock(&words[1])?;
    Ok((day * DAY + minutes * MINUTE - i64::from(zone) * MINUTE, zone))
}

fn parse_date(word: &str) -> Result<i64, String> {
    let parts: Vec<i64> = word.split('-').filter_map(|p| p.parse().ok()).collect();
    let [y, m, d] = parts[..] else { return Err(USAGE.to_owned()) };
    let days = days_from_civil(y, m, d);
    if !(1..=12).contains(&m) || civil_from_days(days) != (y, m, d) || !(1970..=9999).contains(&y) {
        return Err(format!("{word} is not a date"));
    }
    Ok(days)
}

/// `20:00`, `8pm`, `8:30pm`, as minutes after midnight.
fn parse_clock(word: &str) -> Result<i64, String> {
    let (body, pm) = match (word.strip_suffix("pm"), word.strip_suffix("am")) {
        (Some(b), _) => (b, Some(true)),
        (_, Some(b)) => (b, Some(false)),
        _ => (word, None),
    };
    let (h, m) = body.split_once(':').unwrap_or((body, "0"));
    let (Ok(mut h), Ok(m)) = (h.parse::<i64>(), m.parse::<i64>()) else { return Err(format!("{word} is not a time")) };
    if let Some(pm) = pm {
        if !(1..=12).contains(&h) {
            return Err(format!("{word} is not a time"));
        }
        h = h % 12 + if pm { 12 } else { 0 };
    }
    if !(0..24).contains(&h) || !(0..60).contains(&m) {
        return Err(format!("{word} is not a time"));
    }
    Ok(h * 60 + m)
}

fn parse_zone(word: &str) -> Result<i32, String> {
    if matches!(word, "utc" | "z" | "gmt") {
        return Ok(0);
    }
    let sign = match word.chars().next() {
        Some('+') => 1,
        Some('-') => -1,
        _ => return Err(format!("{word} is not a zone; use utc, +02:00 or -05:00")),
    };
    let (h, m) = word[1..].split_once(':').unwrap_or((&word[1..], "0"));
    match (h.parse::<i32>(), m.parse::<i32>()) {
        (Ok(h), Ok(m)) if h <= 14 && m < 60 => Ok(sign * (h * 60 + m)),
        _ => Err(format!("{word} is not a zone; use utc, +02:00 or -05:00")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_764_000_000_000; // 2025-11-24 16:00 UTC

    #[test]
    fn civil_dates_round_trip_across_leap_years() {
        for (y, m, d) in [(1970, 1, 1), (2024, 2, 29), (2026, 12, 5), (2100, 3, 1)] {
            assert_eq!(civil_from_days(days_from_civil(y, m, d)), (y, m, d));
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
    }

    #[test]
    fn a_time_is_shown_in_the_zone_it_was_typed_in() {
        let (at, zone) = parse_when("2026-01-05 20:00 +02:00", NOW).unwrap();
        assert_eq!(at, 1_767_636_000_000);
        assert_eq!(format_local(at, zone), "Mon 5 Jan 2026, 20:00 UTC+02:00");
        assert_eq!(format_local(at, 0), "Mon 5 Jan 2026, 18:00 UTC");
        assert_eq!(format_local(at, -330), "Mon 5 Jan 2026, 12:30 UTC-05:30");
    }

    #[test]
    fn relative_and_twelve_hour_forms_parse() {
        assert_eq!(parse_when("in 1h30m", NOW).unwrap(), (NOW + 90 * MINUTE, 0));
        assert_eq!(parse_when("in 2 days", NOW).unwrap().0, NOW + 2 * DAY);
        assert_eq!(parse_when("tomorrow 8pm", NOW).unwrap().0, 1_764_100_800_000);
        assert_eq!(parse_when("2025-12-01 12am utc", NOW).unwrap().0, 1_764_547_200_000);
        assert_eq!(parse_when("2025-12-01 12:30pm", NOW).unwrap().0, 1_764_592_200_000);
    }

    #[test]
    fn tomorrow_follows_the_typed_zone_not_utc() {
        let late = NOW + 9 * 3_600_000; // 01:00 UTC on the 25th, still the 24th at -05:00
        let (at, _) = parse_when("today 9pm -05:00", late).unwrap();
        assert_eq!(format_local(at, -300), "Mon 24 Nov 2025, 21:00 UTC-05:00");
    }

    #[test]
    fn bad_times_say_what_is_wrong() {
        for bad in ["", "soon", "in", "in 0m", "in 5", "in 3x", "2026-02-30 10:00", "2026-13-01 10:00", "2026-12-05 25:00", "2026-12-05 13pm",
            "2026-12-05 10:00 +15:00", "2026-12-05 10:00 est", "2026-12-05", "2026-12-05 10:00 utc extra", "in 9999d"] {
            assert!(parse_when(bad, NOW).is_err(), "{bad:?} should not parse");
        }
        assert_eq!(parse_when("2025-11-24 15:00", NOW).unwrap_err(), "that time has already passed");
        assert_eq!(parse_when("2030-01-01 10:00", NOW).unwrap_err(), "events can be at most a year ahead");
    }

    #[test]
    fn countdowns_read_naturally() {
        assert_eq!(countdown(30_000), "starting now");
        assert_eq!(countdown(14 * MINUTE), "starts in 14m");
        assert_eq!(countdown(-5 * MINUTE), "started 5m ago");
        assert_eq!(countdown(50 * 3_600_000), "starts in 2d 2h");
        assert_eq!(countdown(90 * MINUTE), "starts in 1h 30m");
    }
}
