//! The one host capability Weather uses, `net.fetch`, reached through the one
//! import slim allows (`slim.host_call`), and the trait the rest of the module
//! talks to so tests can answer from memory.

pub trait Host {
    fn get(&self, url: &str) -> Result<Fetched, FetchError>;
}

/// A response the host handed back: its status and the body as text.
pub struct Fetched {
    pub status: u16,
    pub body: String,
}

/// Why a fetch did not produce a response, sorted by what the reader can do about it.
#[derive(Debug, PartialEq)]
pub enum FetchError {
    NotApproved,
    TimedOut,
    Other(String),
}

impl FetchError {
    pub fn classify(host_error: &str) -> Self {
        if host_error.contains("not approved") && host_error.contains("capability") {
            FetchError::NotApproved
        } else if host_error.contains("timed out") {
            FetchError::TimedOut
        } else {
            FetchError::Other(host_error.to_string())
        }
    }

    pub fn text(&self) -> String {
        match self {
            FetchError::NotApproved => "An admin needs to allow Weather to reach the internet in the Dock.".to_string(),
            FetchError::TimedOut => "The weather service is too slow right now. Try again in a minute.".to_string(),
            FetchError::Other(detail) => format!("Weather could not reach its service ({detail})."),
        }
    }
}

/// The real host: a GET that slim sends to a site the manifest names and an admin approved.
pub struct SlimHost;

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "slim")]
extern "C" {
    fn host_call(ptr: i32, len: i32) -> i64;
}

#[cfg(target_arch = "wasm32")]
fn call(request: &serde_json::Value) -> Result<serde_json::Value, String> {
    let bytes = serde_json::to_vec(request).map_err(|_| "could not build the request".to_string())?;
    let packed = unsafe { host_call(bytes.as_ptr() as i32, bytes.len() as i32) };
    // slim answers 0 when the run is past its deadline or the reply could not be written.
    if packed == 0 {
        return Err("slim did not answer in time".to_string());
    }
    let (ptr, len) = ((packed >> 32) as u32 as usize, (packed & 0xffff_ffff) as usize);
    let reply = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
    let reply: serde_json::Value = serde_json::from_slice(reply).map_err(|_| "slim answered nonsense".to_string())?;
    if reply["ok"] == true {
        Ok(reply)
    } else {
        Err(reply["error"].as_str().unwrap_or("slim refused").to_string())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn call(_request: &serde_json::Value) -> Result<serde_json::Value, String> {
    Err("host capabilities only exist inside slim".to_string())
}

impl Host for SlimHost {
    fn get(&self, url: &str) -> Result<Fetched, FetchError> {
        let reply = call(&serde_json::json!({"capability": "net.fetch", "method": "GET", "url": url}))
            .map_err(|e| FetchError::classify(&e))?;
        Ok(Fetched {
            status: reply["status"].as_u64().unwrap_or(0) as u16,
            body: reply["body"].as_str().unwrap_or_default().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_errors_are_sorted_by_what_the_reader_can_do() {
        assert_eq!(FetchError::classify("capability not approved: net.fetch"), FetchError::NotApproved);
        assert_eq!(FetchError::classify("net.fetch timed out"), FetchError::TimedOut);
        assert!(matches!(FetchError::classify("host not approved for this module: x.test"), FetchError::Other(_)));
    }

    #[test]
    fn outside_slim_the_real_host_answers_an_error_instead_of_trapping() {
        assert!(SlimHost.get("https://api.open-meteo.com/").is_err());
    }
}
