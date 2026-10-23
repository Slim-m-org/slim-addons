//! The one import slim allows (`slim.host_call`), behind a trait so tests can swap in memory.

use serde_json::Value;

pub trait Host {
    /// Sends one capability request; `Err` carries the host's refusal text.
    fn call(&self, request: &Value) -> Result<Value, String>;
}

pub struct SlimHost;

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "slim")]
extern "C" {
    fn host_call(ptr: i32, len: i32) -> i64;
}

impl Host for SlimHost {
    #[cfg(target_arch = "wasm32")]
    fn call(&self, request: &Value) -> Result<Value, String> {
        let bytes = serde_json::to_vec(request).map_err(|_| "could not build the request".to_string())?;
        let packed = unsafe { host_call(bytes.as_ptr() as i32, bytes.len() as i32) };
        // slim answers 0 when the run is past its deadline or the reply could not be written.
        if packed == 0 {
            return Err("slim did not answer in time".to_string());
        }
        let (ptr, len) = ((packed >> 32) as u32 as usize, (packed & 0xffff_ffff) as usize);
        let reply = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
        let reply: Value = serde_json::from_slice(reply).map_err(|_| "slim answered nonsense".to_string())?;
        if reply["ok"] == true {
            Ok(reply)
        } else {
            Err(reply["error"].as_str().unwrap_or("slim refused").to_string())
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn call(&self, _request: &Value) -> Result<Value, String> {
        Err("host capabilities only exist inside slim".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outside_slim_the_real_host_answers_an_error_instead_of_trapping() {
        assert!(SlimHost.call(&serde_json::json!({"capability": "log.read"})).is_err());
    }
}
