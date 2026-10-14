//! An in-memory host for tests that enforces the same `kv.store` limits as slim,
//! so a test can hold a command to the entry, byte and per-run call budgets.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use crate::host::{Host, KvError, PostError};

const MAX_KEY: usize = 256;
const MAX_VALUE: usize = 4 * 1024;
const MAX_ENTRIES: usize = 256;
const MAX_TOTAL: usize = 64 * 1024;
const MAX_CALLS: u32 = 64;

#[derive(Default)]
pub struct FakeHost {
    pub map: RefCell<BTreeMap<String, String>>,
    pub calls: Cell<u32>,
    pub kv_off: Cell<bool>,
    pub post_mode: RefCell<Option<PostError>>,
    pub posted: RefCell<Vec<String>>,
}

impl FakeHost {
    fn enter(&self) -> Result<(), KvError> {
        self.calls.set(self.calls.get() + 1);
        if self.kv_off.get() {
            return Err(KvError::NotApproved);
        }
        if self.calls.get() > MAX_CALLS {
            return Err(KvError::Other("kv.store call budget for this run is exhausted".to_string()));
        }
        Ok(())
    }

    /// Starts a new run: the call budget resets, the stored data stays.
    pub fn next_run(&self) {
        self.calls.set(0);
    }

    pub fn total_bytes(&self) -> usize {
        self.map.borrow().iter().map(|(k, v)| k.len() + v.len()).sum()
    }
}

impl Host for FakeHost {
    fn get(&self, key: &str) -> Result<Option<String>, KvError> {
        self.enter()?;
        Ok(self.map.borrow().get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), KvError> {
        self.enter()?;
        assert!(key.len() <= MAX_KEY, "key of {} bytes", key.len());
        assert!(value.len() <= MAX_VALUE, "value of {} bytes", value.len());
        let mut map = self.map.borrow_mut();
        let before: usize = map.get(key).map_or(0, |v| key.len() + v.len());
        let entries = map.len() + usize::from(!map.contains_key(key));
        let total = map.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>() - before + key.len() + value.len();
        if entries > MAX_ENTRIES || total > MAX_TOTAL {
            return Err(KvError::Full);
        }
        map.insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), KvError> {
        self.enter()?;
        self.map.borrow_mut().remove(key);
        Ok(())
    }

    fn post(&self, content: &str) -> Result<(), PostError> {
        match self.post_mode.borrow_mut().take() {
            Some(error) => Err(error),
            None => {
                self.posted.borrow_mut().push(content.to_string());
                Ok(())
            }
        }
    }
}
