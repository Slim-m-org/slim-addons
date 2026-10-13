//! The `kv.store` host capability, reached through the one import slim allows
//! (`slim.host_call`), and the trait the sheet code talks to so tests can swap
//! in memory.

pub trait Kv {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

/// The host's store: private to this module, durable across runs.
pub struct HostKv;

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "slim")]
extern "C" {
    fn host_call(ptr: i32, len: i32) -> i64;
}

#[cfg(target_arch = "wasm32")]
fn call(request: &serde_json::Value) -> Result<serde_json::Value, String> {
    let bytes = serde_json::to_vec(request).map_err(|_| "could not build the store request".to_string())?;
    let packed = unsafe { host_call(bytes.as_ptr() as i32, bytes.len() as i32) };
    // slim answers 0 when the run is past its deadline or the reply could not be written.
    if packed == 0 {
        return Err("the store did not answer in time".to_string());
    }
    let (ptr, len) = ((packed >> 32) as u32 as usize, (packed & 0xffff_ffff) as usize);
    let reply = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
    let reply: serde_json::Value = serde_json::from_slice(reply).map_err(|_| "the store answered nonsense".to_string())?;
    if reply["ok"] == true {
        Ok(reply)
    } else {
        Err(reply["error"].as_str().unwrap_or("the store refused").to_string())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn call(_request: &serde_json::Value) -> Result<serde_json::Value, String> {
    Err("kv.store only exists inside slim".to_string())
}

impl Kv for HostKv {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        let reply = call(&serde_json::json!({"capability": "kv.store", "op": "get", "key": key}))?;
        Ok(reply["value"].as_str().map(str::to_string))
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        call(&serde_json::json!({"capability": "kv.store", "op": "set", "key": key, "value": value})).map(|_| ())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        call(&serde_json::json!({"capability": "kv.store", "op": "delete", "key": key})).map(|_| ())
    }
}

#[cfg(test)]
pub use memory::MemKv;

#[cfg(test)]
mod memory {
    use super::Kv;
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;

    /// An in-memory store that counts calls, so a test can hold a command to the per-run budget.
    #[derive(Default)]
    pub struct MemKv {
        pub map: RefCell<BTreeMap<String, String>>,
        pub calls: Cell<u32>,
        pub down: Cell<bool>,
    }

    impl MemKv {
        fn enter(&self) -> Result<(), String> {
            self.calls.set(self.calls.get() + 1);
            if self.down.get() {
                return Err("kv.store capability not approved".to_string());
            }
            Ok(())
        }
    }

    impl Kv for MemKv {
        fn get(&self, key: &str) -> Result<Option<String>, String> {
            self.enter()?;
            Ok(self.map.borrow().get(key).cloned())
        }

        fn set(&self, key: &str, value: &str) -> Result<(), String> {
            self.enter()?;
            self.map.borrow_mut().insert(key.to_string(), value.to_string());
            Ok(())
        }

        fn delete(&self, key: &str) -> Result<(), String> {
            self.enter()?;
            self.map.borrow_mut().remove(key);
            Ok(())
        }
    }
}
