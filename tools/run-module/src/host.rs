//! The one host function slim lets a module import, `slim.host_call`, with a
//! `kv.store` behind it that enforces slim's own bounds. It is a stand-in for
//! trying a module out: a capability is always approved here, and the store
//! lives in memory unless `--kv-file` keeps it between runs.

use std::collections::BTreeMap;

use wasmi::{Caller, Extern, Linker, Memory};

const MAX_KEY_BYTES: usize = 256;
const MAX_VALUE_BYTES: usize = 4 * 1024;
const MAX_ENTRIES: usize = 256;
const MAX_TOTAL_BYTES: usize = 64 * 1024;
const MAX_CALLS_PER_RUN: u32 = 64;

#[derive(Default)]
pub struct Host {
    pub kv: BTreeMap<String, String>,
    calls: u32,
}

pub fn link(linker: &mut Linker<Host>) -> Result<(), String> {
    linker
        .func_wrap("slim", "host_call", host_call)
        .map(|_| ())
        .map_err(|e| format!("cannot link host_call: {e}"))
}

fn host_call(mut caller: Caller<'_, Host>, ptr: i32, len: i32) -> i64 {
    let Some(Extern::Memory(memory)) = caller.get_export("memory") else { return 0 };
    let mut request = vec![0u8; len.max(0) as usize];
    if memory.read(&caller, ptr as u32 as usize, &mut request).is_err() {
        return 0;
    }
    let reply = handle(caller.data_mut(), &String::from_utf8_lossy(&request));
    write_reply(&mut caller, &memory, reply.as_bytes()).unwrap_or(0)
}

fn write_reply(caller: &mut Caller<'_, Host>, memory: &Memory, reply: &[u8]) -> Option<i64> {
    let Some(Extern::Func(alloc)) = caller.get_export("alloc") else { return None };
    let alloc = alloc.typed::<i32, i32>(&*caller).ok()?;
    let ptr = alloc.call(&mut *caller, reply.len() as i32).ok()?;
    memory.write(&mut *caller, ptr as u32 as usize, reply).ok()?;
    Some(((ptr as u32 as i64) << 32) | reply.len() as i64)
}

fn refusal(message: &str) -> String {
    format!("{{\"ok\":false,\"error\":\"{message}\"}}")
}

/// Answers one `kv.store` request. The reading is deliberately dumb (string
/// search, no JSON parser) to keep this tool dependency-light; slim's own
/// handler is the authority on the wire shape.
pub fn handle(host: &mut Host, request: &str) -> String {
    if host.calls >= MAX_CALLS_PER_RUN {
        return refusal("kv.store call budget for this run is exhausted");
    }
    host.calls += 1;
    if field(request, "capability").as_deref() != Some("kv.store") {
        return refusal("capability not available");
    }
    let (key, value) = (field(request, "key"), field(request, "value"));
    match (field(request, "op").as_deref(), key, value) {
        (Some("get"), Some(key), _) => match host.kv.get(&key) {
            Some(value) => format!("{{\"ok\":true,\"value\":\"{}\"}}", crate::escape(value)),
            None => "{\"ok\":true,\"value\":null}".to_owned(),
        },
        (Some("set"), Some(key), Some(value)) => set(host, key, value),
        (Some("delete"), Some(key), _) => {
            host.kv.remove(&key);
            "{\"ok\":true}".to_owned()
        }
        _ => refusal("malformed kv.store request"),
    }
}

fn set(host: &mut Host, key: String, value: String) -> String {
    if key.len() > MAX_KEY_BYTES {
        return refusal("kv.store key is too long");
    }
    if value.len() > MAX_VALUE_BYTES {
        return refusal("kv.store value is too large");
    }
    let others: usize = host.kv.iter().filter(|(k, _)| **k != key).map(|(k, v)| k.len() + v.len()).sum();
    let entries = host.kv.len() + usize::from(!host.kv.contains_key(&key));
    if entries > MAX_ENTRIES || others + key.len() + value.len() > MAX_TOTAL_BYTES {
        return refusal("kv.store is full for this module");
    }
    host.kv.insert(key, value);
    "{\"ok\":true}".to_owned()
}

/// The string value of `"name":"..."` in a flat JSON object, with escapes undone.
fn field(request: &str, name: &str) -> Option<String> {
    let rest = &request[request.find(&format!("\"{name}\":\""))? + name.len() + 4..];
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

/// One `hexkey hexvalue` line per entry, so any bytes survive a text file.
pub fn dump(kv: &BTreeMap<String, String>) -> String {
    let hex = |s: &str| s.bytes().map(|b| format!("{b:02x}")).collect::<String>();
    kv.iter().map(|(k, v)| format!("{} {}\n", hex(k), hex(v))).collect()
}

pub fn load(text: &str) -> BTreeMap<String, String> {
    let unhex = |s: &str| {
        let bytes: Vec<u8> = (0..s.len() / 2).filter_map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    text.lines().filter_map(|line| line.split_once(' ')).map(|(k, v)| (unhex(k), unhex(v))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(host: &mut Host, request: &str) -> String {
        handle(host, request)
    }

    #[test]
    fn set_get_and_delete_round_trip_with_escapes() {
        let mut host = Host::default();
        assert_eq!(call(&mut host, r#"{"capability":"kv.store","op":"set","key":"k","value":"a\"b\nc"}"#), r#"{"ok":true}"#);
        assert_eq!(call(&mut host, r#"{"capability":"kv.store","op":"get","key":"k"}"#), r#"{"ok":true,"value":"a\"b\nc"}"#);
        call(&mut host, r#"{"capability":"kv.store","op":"delete","key":"k"}"#);
        assert_eq!(call(&mut host, r#"{"capability":"kv.store","op":"get","key":"k"}"#), r#"{"ok":true,"value":null}"#);
    }

    #[test]
    fn slims_bounds_are_enforced() {
        let mut host = Host::default();
        let big = "x".repeat(MAX_VALUE_BYTES + 1);
        let request = format!(r#"{{"capability":"kv.store","op":"set","key":"k","value":"{big}"}}"#);
        assert!(call(&mut host, &request).contains("too large"));
        let mut host = Host::default();
        for _ in 0..MAX_CALLS_PER_RUN {
            call(&mut host, r#"{"capability":"kv.store","op":"get","key":"k"}"#);
        }
        assert!(call(&mut host, r#"{"capability":"kv.store","op":"get","key":"k"}"#).contains("budget"));
        assert!(call(&mut Host::default(), r#"{"capability":"message.post"}"#).contains("not available"));
    }

    #[test]
    fn the_store_survives_a_dump_and_load() {
        let kv = BTreeMap::from([("sheet:a".to_owned(), "{\"n\":\"Aria\"}".to_owned())]);
        assert_eq!(load(&dump(&kv)), kv);
    }
}
