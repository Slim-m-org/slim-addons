//! The host capabilities Reminder uses, reached through the one import slim
//! allows (`slim.host_call`), and the trait the rest of the module talks to so
//! tests can swap in memory.

pub trait Host {
    fn get(&self, key: &str) -> Result<Option<String>, KvError>;
    fn set(&self, key: &str, value: &str) -> Result<(), KvError>;
    fn delete(&self, key: &str) -> Result<(), KvError>;
    fn post(&self, content: &str) -> Result<(), PostError>;
}

/// Why `kv.store` said no, sorted by what the person reading the error can do about it.
#[derive(Debug, PartialEq)]
pub enum KvError {
    NotApproved,
    Full,
    Other(String),
}

impl KvError {
    pub fn classify(host_error: &str) -> Self {
        if host_error.contains("not approved") {
            KvError::NotApproved
        } else if host_error.contains("full") {
            KvError::Full
        } else {
            KvError::Other(host_error.to_string())
        }
    }

    pub fn text(&self) -> String {
        match self {
            KvError::NotApproved => "An admin needs to allow Reminder to store data in the Dock.".to_string(),
            KvError::Full => "Reminder has no room left. Cancel one first.".to_string(),
            KvError::Other(detail) => format!("Reminder could not reach its storage ({detail}). Try again."),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum PostError {
    /// Not approved, or no channel to post into: the caller falls back to plain output.
    Unavailable,
    Refused(String),
}

impl PostError {
    pub fn classify(host_error: &str) -> Self {
        let unavailable = ["not approved", "needs a channel", "not available"];
        if unavailable.iter().any(|marker| host_error.contains(marker)) {
            PostError::Unavailable
        } else {
            PostError::Refused(host_error.to_string())
        }
    }
}

/// The real host: `kv.store` is private to this module, `message.post` speaks as whoever ran it.
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

fn kv_call(request: serde_json::Value) -> Result<serde_json::Value, KvError> {
    call(&request).map_err(|e| KvError::classify(&e))
}

impl Host for SlimHost {
    fn get(&self, key: &str) -> Result<Option<String>, KvError> {
        let reply = kv_call(serde_json::json!({"capability": "kv.store", "op": "get", "key": key}))?;
        Ok(reply["value"].as_str().map(str::to_string))
    }

    fn set(&self, key: &str, value: &str) -> Result<(), KvError> {
        kv_call(serde_json::json!({"capability": "kv.store", "op": "set", "key": key, "value": value})).map(|_| ())
    }

    fn delete(&self, key: &str) -> Result<(), KvError> {
        kv_call(serde_json::json!({"capability": "kv.store", "op": "delete", "key": key})).map(|_| ())
    }

    fn post(&self, content: &str) -> Result<(), PostError> {
        call(&serde_json::json!({"capability": "message.post", "content": content}))
            .map(|_| ())
            .map_err(|e| PostError::classify(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_kv_errors_are_sorted_by_what_the_reader_can_do() {
        assert_eq!(KvError::classify("capability not approved: kv.store"), KvError::NotApproved);
        assert_eq!(KvError::classify("kv.store is full for this module"), KvError::Full);
        assert!(matches!(KvError::classify("kv.store call budget for this run is exhausted"), KvError::Other(_)));
    }

    #[test]
    fn a_missing_post_capability_is_told_apart_from_a_refused_post() {
        for gone in ["capability not approved: message.post", "message.post needs a channel", "capability not available"] {
            assert_eq!(PostError::classify(gone), PostError::Unavailable);
        }
        assert_eq!(
            PostError::classify("message.post rate limit reached"),
            PostError::Refused("message.post rate limit reached".to_string())
        );
        assert!(matches!(PostError::classify("message.post is unavailable"), PostError::Refused(_)));
    }

    #[test]
    fn outside_slim_the_real_host_answers_an_error_instead_of_trapping() {
        assert!(SlimHost.get("k").is_err());
        assert!(SlimHost.post("hi").is_err());
    }
}
