//! The one import slim allows (`slim.host_call`), behind a trait so tests can swap in memory.

use serde_json::{json, Value};

pub trait Host {
    /// Sends one capability request; `Err` carries the host's refusal text.
    fn call(&self, request: &Value) -> Result<Value, String>;

    fn get(&self, key: &str) -> Result<Option<String>, String> {
        let reply = self.call(&json!({"capability": "kv.store", "op": "get", "key": key}))?;
        Ok(reply["value"].as_str().map(str::to_owned))
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.call(&json!({"capability": "kv.store", "op": "set", "key": key, "value": value})).map(|_| ())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        self.call(&json!({"capability": "kv.store", "op": "delete", "key": key})).map(|_| ())
    }

    fn keys(&self) -> Result<Vec<String>, String> {
        let reply = self.call(&json!({"capability": "kv.store", "op": "list"}))?;
        Ok(reply["keys"].as_array().map(|keys| keys.iter().filter_map(|k| k.as_str().map(str::to_owned)).collect()).unwrap_or_default())
    }

    fn post(&self, content: &str) -> Result<(), String> {
        self.call(&json!({"capability": "message.post", "content": content})).map(|_| ())
    }
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
pub mod fake {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// An in-memory host that records every post it is asked for.
    #[derive(Default)]
    pub struct FakeHost {
        pub kv: RefCell<BTreeMap<String, String>>,
        pub posts: RefCell<Vec<String>>,
        pub refuse_posts: bool,
    }

    impl Host for FakeHost {
        fn call(&self, request: &Value) -> Result<Value, String> {
            let key = request["key"].as_str().unwrap_or("").to_owned();
            match (request["capability"].as_str(), request["op"].as_str()) {
                (Some("kv.store"), Some("get")) => Ok(json!({"ok": true, "value": self.kv.borrow().get(&key)})),
                (Some("kv.store"), Some("set")) => {
                    let value = request["value"].as_str().unwrap_or("");
                    if value.len() > 4096 {
                        return Err("kv.store value is too large".to_owned());
                    }
                    self.kv.borrow_mut().insert(key, value.to_owned());
                    Ok(json!({"ok": true}))
                }
                (Some("kv.store"), Some("list")) => Ok(json!({"ok": true, "keys": self.kv.borrow().keys().collect::<Vec<_>>()})),
                (Some("kv.store"), Some("delete")) => {
                    self.kv.borrow_mut().remove(&key);
                    Ok(json!({"ok": true}))
                }
                (Some("message.post"), _) if self.refuse_posts => Err("message.post needs a channel".to_owned()),
                (Some("message.post"), _) => {
                    self.posts.borrow_mut().push(request["content"].as_str().unwrap_or("").to_owned());
                    Ok(json!({"ok": true, "message_id": "m"}))
                }
                _ => Err("malformed request".to_owned()),
            }
        }
    }

    #[test]
    fn outside_slim_the_real_host_answers_an_error_instead_of_trapping() {
        assert!(SlimHost.get("k").is_err());
        assert!(SlimHost.post("hi").is_err());
    }
}
