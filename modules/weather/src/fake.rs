//! An in-memory host for tests: answers the two Open-Meteo requests with
//! canned bodies and records every URL it was asked for.

use std::cell::RefCell;

use crate::host::{FetchError, Fetched, Host};

pub struct FakeHost {
    pub seen: RefCell<Vec<String>>,
    pub failure: RefCell<Option<FetchError>>,
    pub status: RefCell<u16>,
    pub body_override: RefCell<Option<String>>,
}

impl Default for FakeHost {
    fn default() -> Self {
        FakeHost {
            seen: RefCell::default(),
            failure: RefCell::default(),
            status: RefCell::new(200),
            body_override: RefCell::default(),
        }
    }
}

const BERLIN: &str = r#"{"results":[{"name":"Berlin","admin1":"Berlin","country":"Germany","latitude":52.52,"longitude":13.405}]}"#;
const NOWHERE: &str = r#"{"generationtime_ms":0.5}"#;
const CONDITIONS: &str = r#"{"current":{"temperature_2m":12.34,"apparent_temperature":10.06,"weather_code":3,"wind_speed_10m":14.2}}"#;

impl Host for FakeHost {
    fn get(&self, url: &str) -> Result<Fetched, FetchError> {
        self.seen.borrow_mut().push(url.to_string());
        if let Some(error) = self.failure.borrow_mut().take() {
            return Err(error);
        }
        let body = match self.body_override.borrow().clone() {
            Some(body) => body,
            None if url.contains("geocoding-api") && url.contains("name=Nowhereville") => NOWHERE.to_string(),
            None if url.contains("geocoding-api") => BERLIN.to_string(),
            None => CONDITIONS.to_string(),
        };
        Ok(Fetched { status: *self.status.borrow(), body })
    }
}
