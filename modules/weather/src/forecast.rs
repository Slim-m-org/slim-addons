//! Turning a place name into a one-line forecast: look the place up, then ask
//! for its current conditions. Two `net.fetch` requests, both to hosts the
//! manifest names.

use serde_json::Value;

use crate::host::{FetchError, Host};

const GEOCODE: &str = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST: &str = "https://api.open-meteo.com/v1/forecast";
/// Longest place name accepted; longer is never a place.
const MAX_PLACE_CHARS: usize = 80;

pub fn weather<H: Host>(host: &H, place: &str) -> Result<String, String> {
    let place = place.trim();
    if place.is_empty() {
        return Err("Say where: /weather Berlin".to_string());
    }
    if place.chars().count() > MAX_PLACE_CHARS {
        return Err("That place name is too long.".to_string());
    }
    let found = lookup(host, place)?;
    let current = conditions(host, &found)?;
    Ok(format!("{}: {}", found.label(), current))
}

struct Place {
    name: String,
    region: Option<String>,
    country: Option<String>,
    latitude: f64,
    longitude: f64,
}

impl Place {
    fn label(&self) -> String {
        let parts = [Some(&self.name), self.region.as_ref(), self.country.as_ref()];
        let mut seen: Vec<&str> = Vec::new();
        for part in parts.into_iter().flatten() {
            if !seen.contains(&part.as_str()) {
                seen.push(part);
            }
        }
        seen.join(", ")
    }
}

fn lookup<H: Host>(host: &H, place: &str) -> Result<Place, String> {
    let url = format!("{GEOCODE}?name={}&count=1&language=en&format=json", encode(place));
    let body = get_json(host, &url)?;
    let hit = body["results"].get(0).ok_or_else(|| format!("No place called {place} was found."))?;
    let text = |key: &str| hit[key].as_str().map(str::to_string);
    Ok(Place {
        name: text("name").ok_or("The weather service answered nonsense.")?,
        region: text("admin1"),
        country: text("country"),
        latitude: hit["latitude"].as_f64().ok_or("The weather service answered nonsense.")?,
        longitude: hit["longitude"].as_f64().ok_or("The weather service answered nonsense.")?,
    })
}

fn conditions<H: Host>(host: &H, place: &Place) -> Result<String, String> {
    let url = format!(
        "{FORECAST}?latitude={:.4}&longitude={:.4}&current=temperature_2m,apparent_temperature,weather_code,wind_speed_10m&timezone=auto",
        place.latitude, place.longitude
    );
    let body = get_json(host, &url)?;
    let now = &body["current"];
    let temperature = now["temperature_2m"].as_f64().ok_or("The weather service gave no temperature.")?;
    let feels = now["apparent_temperature"].as_f64();
    let wind = now["wind_speed_10m"].as_f64();
    let mut line = format!("{}, {temperature:.1}\u{b0}C", describe(now["weather_code"].as_u64().unwrap_or(u64::MAX)));
    if let Some(feels) = feels {
        line.push_str(&format!(" (feels like {feels:.1}\u{b0}C)"));
    }
    if let Some(wind) = wind {
        line.push_str(&format!(", wind {wind:.0} km/h"));
    }
    Ok(line)
}

fn get_json<H: Host>(host: &H, url: &str) -> Result<Value, String> {
    let fetched = host.get(url).map_err(|e: FetchError| e.text())?;
    if fetched.status != 200 {
        return Err(format!("The weather service answered {}.", fetched.status));
    }
    serde_json::from_str(&fetched.body).map_err(|_| "The weather service answered nonsense.".to_string())
}

/// WMO weather interpretation codes, as Open-Meteo reports them.
fn describe(code: u64) -> &'static str {
    match code {
        0 => "clear sky",
        1 => "mostly clear",
        2 => "partly cloudy",
        3 => "overcast",
        45 | 48 => "fog",
        51..=57 => "drizzle",
        61..=67 => "rain",
        71..=77 => "snow",
        80..=82 => "rain showers",
        85 | 86 => "snow showers",
        95..=99 => "thunderstorm",
        _ => "unsettled",
    }
}

/// Percent-encodes everything but unreserved characters, byte by byte, so any
/// place name is one safe query value.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeHost;

    #[test]
    fn a_place_becomes_a_one_line_forecast_from_two_requests() {
        let host = FakeHost::default();
        let line = weather(&host, "Berlin").unwrap();
        assert_eq!(line, "Berlin, Germany: overcast, 12.3\u{b0}C (feels like 10.1\u{b0}C), wind 14 km/h");
        let seen = host.seen.borrow();
        assert_eq!(seen.len(), 2);
        assert!(seen[0].starts_with("https://geocoding-api.open-meteo.com/v1/search?name=Berlin&"), "{}", seen[0]);
        assert!(seen[1].starts_with("https://api.open-meteo.com/v1/forecast?latitude=52.5200&longitude=13.4050&"), "{}", seen[1]);
    }

    #[test]
    fn a_place_name_is_encoded_into_one_query_value() {
        let host = FakeHost::default();
        weather(&host, "Z\u{fc}rich & more").ok();
        assert!(host.seen.borrow()[0].contains("name=Z%C3%BCrich%20%26%20more&"), "{}", host.seen.borrow()[0]);
        assert_eq!(encode("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn a_missing_place_empty_input_and_a_long_name_are_told_plainly() {
        let host = FakeHost::default();
        assert_eq!(weather(&host, "Nowhereville").unwrap_err(), "No place called Nowhereville was found.");
        assert!(weather(&host, "  ").unwrap_err().contains("Say where"));
        assert!(weather(&host, &"x".repeat(81)).unwrap_err().contains("too long"));
        assert!(host.seen.borrow().len() == 1, "only the lookup for the missing place was made");
    }

    #[test]
    fn a_host_failure_is_explained_in_the_readers_terms() {
        let host = FakeHost::default();
        *host.failure.borrow_mut() = Some(FetchError::NotApproved);
        assert!(weather(&host, "Berlin").unwrap_err().contains("An admin needs to allow Weather"));
        *host.failure.borrow_mut() = Some(FetchError::TimedOut);
        assert!(weather(&host, "Berlin").unwrap_err().contains("too slow"));
    }

    #[test]
    fn a_service_that_errors_or_babbles_is_not_trusted() {
        let host = FakeHost::default();
        *host.status.borrow_mut() = 503;
        assert_eq!(weather(&host, "Berlin").unwrap_err(), "The weather service answered 503.");
        *host.status.borrow_mut() = 200;
        *host.body_override.borrow_mut() = Some("not json".to_string());
        assert_eq!(weather(&host, "Berlin").unwrap_err(), "The weather service answered nonsense.");
    }
}
