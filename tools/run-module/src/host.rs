//! The one host function slim lets a module import, `slim.host_call`, with the
//! capabilities behind it: `kv.store` held in memory (`--kv-file` keeps it
//! between runs), `net.fetch` answered from canned responses (`--fetch-file`,
//! see `net.rs`), `message.post` recorded and printed instead of sent,
//! `data.store` held in memory (`data.rs`) and `log.read` served from a file
//! (`logs.rs`). It is a stand-in for trying a module out: a capability is
//! always approved here.

use std::collections::BTreeMap;

use wasmi::{Caller, Extern, Linker, Memory};

use crate::data::Data;
use crate::logs::Logs;
use crate::net::Canned;

const MAX_KEY_BYTES: usize = 256;
const MAX_VALUE_BYTES: usize = 4 * 1024;
const MAX_ENTRIES: usize = 256;
const MAX_TOTAL_BYTES: usize = 64 * 1024;
const MAX_CALLS_PER_RUN: u32 = 64;
/// Host calls of any kind in one run, refused ones included.
const MAX_HOST_CALLS_PER_RUN: u32 = 128;
const MAX_POSTS_PER_RUN: u32 = 3;
/// Largest request slim reads from a module; a bigger one is refused before anything is copied.
const MAX_HOST_CALL_REQUEST_BYTES: usize = 128 * 1024;

#[derive(Default)]
pub struct Host {
    pub kv: BTreeMap<String, String>,
    /// The canned responses `net.fetch` answers from; absent without `--fetch-file`.
    pub fetch: Option<Canned>,
    /// Every `message.post` content the run made, in order.
    pub posts: Vec<String>,
    /// The manifest's tables; absent when the module declares none.
    pub data: Option<Data>,
    /// The logs `log.read` serves; absent without `--log-file`.
    pub logs: Option<Logs>,
    calls: u32,
    all_calls: u32,
}

pub fn link(linker: &mut Linker<Host>) -> Result<(), String> {
    linker
        .func_wrap("slim", "host_call", host_call)
        .map(|_| ())
        .map_err(|e| format!("cannot link host_call: {e}"))
}

fn host_call(mut caller: Caller<'_, Host>, ptr: i32, len: i32) -> i64 {
    let Some(Extern::Memory(memory)) = caller.get_export("memory") else { return 0 };
    let Some((ptr, len)) = request_span(ptr, len) else { return 0 };
    let mut request = vec![0u8; len];
    if memory.read(&caller, ptr, &mut request).is_err() {
        return 0;
    }
    let reply = handle(caller.data_mut(), &String::from_utf8_lossy(&request));
    write_reply(&mut caller, &memory, reply.as_bytes()).unwrap_or(0)
}

/// Where a request lies in the module's memory, or `None` where slim answers nothing.
fn request_span(ptr: i32, len: i32) -> Option<(usize, usize)> {
    let len = usize::try_from(len).ok().filter(|len| *len <= MAX_HOST_CALL_REQUEST_BYTES)?;
    Some((usize::try_from(ptr).ok()?, len))
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

/// Routes one `host_call` request to the capability it names.
pub fn handle(host: &mut Host, request: &str) -> String {
    if host.all_calls >= MAX_HOST_CALLS_PER_RUN {
        return refusal("host_call budget for this run is exhausted");
    }
    host.all_calls += 1;
    match field(request, "capability").as_deref() {
        Some("kv.store") => kv(host, request),
        Some("net.fetch") => match host.fetch.as_mut() {
            Some(canned) => canned.handle(request),
            None => refusal("net.fetch has no canned responses here; pass --fetch-file"),
        },
        Some("message.post") => post(host, request),
        Some("data.store") => data_store(host, request),
        Some("log.read") => log_read(host, request),
        _ => refusal("capability not available"),
    }
}

/// Spends one of the store calls `kv.store` and `data.store` share; `Some` is the refusal.
fn spend_store_call(host: &mut Host, capability: &str) -> Option<String> {
    if host.calls >= MAX_CALLS_PER_RUN {
        return Some(refusal(&format!("{capability} call budget for this run is exhausted")));
    }
    host.calls += 1;
    None
}

fn data_store(host: &mut Host, request: &str) -> String {
    if let Some(refused) = spend_store_call(host, "data.store") {
        return refused;
    }
    let Ok(parsed) = serde_json::from_str(request) else { return refusal("malformed data.store request") };
    match host.data.as_mut() {
        Some(data) => data.handle(&parsed).to_string(),
        None => refusal("this module declared no data; pass --manifest if it is not beside the wasm"),
    }
}

fn log_read(host: &Host, request: &str) -> String {
    let Ok(parsed) = serde_json::from_str(request) else { return refusal("malformed log.read request") };
    match host.logs.as_ref() {
        Some(logs) => logs.handle(&parsed).to_string(),
        None => refusal("log.read has no logs here; pass --log-file"),
    }
}

/// Records a post; slim would send it as the person who ran the module.
fn post(host: &mut Host, request: &str) -> String {
    if host.posts.len() as u32 >= MAX_POSTS_PER_RUN {
        return refusal("message.post budget for this run is exhausted");
    }
    match field(request, "content") {
        Some(content) if !content.trim().is_empty() => {
            host.posts.push(content);
            format!("{{\"ok\":true,\"message_id\":\"simulated-{}\"}}", host.posts.len())
        }
        _ => refusal("message.post needs content"),
    }
}

/// Answers one `kv.store` request. The reading is deliberately dumb (string
/// search, no JSON parser) to keep it small; slim's own handler is the
/// authority on the wire shape.
fn kv(host: &mut Host, request: &str) -> String {
    if let Some(refused) = spend_store_call(host, "kv.store") {
        return refused;
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
        assert!(call(&mut Host::default(), r#"{"capability":"database"}"#).contains("not available"));
    }

    #[test]
    fn a_post_is_recorded_not_refused_and_is_capped_per_run() {
        let mut host = Host::default();
        let reply = call(&mut host, r#"{"capability":"message.post","content":"hello"}"#);
        assert!(reply.contains(r#""ok":true"#) && reply.contains("simulated-1"), "{reply}");
        assert_eq!(host.posts, ["hello"]);
        assert!(call(&mut host, r#"{"capability":"message.post"}"#).contains("needs content"));
        for _ in 0..MAX_POSTS_PER_RUN {
            call(&mut host, r#"{"capability":"message.post","content":"again"}"#);
        }
        assert!(call(&mut host, r#"{"capability":"message.post","content":"x"}"#).contains("budget"));
    }

    #[test]
    fn data_store_and_kv_share_one_budget_of_store_calls() {
        let manifest = serde_json::json!({"data": {"tables": [{"name": "t", "columns": [{"name": "n", "type": "int"}]}]}});
        let mut host = Host { data: Data::from_manifest(&manifest), ..Host::default() };
        let count = r#"{"capability":"data.store","op":"count","table":"t"}"#;
        for _ in 0..MAX_CALLS_PER_RUN / 2 {
            assert!(call(&mut host, count).contains(r#""ok":true"#));
            call(&mut host, r#"{"capability":"kv.store","op":"get","key":"k"}"#);
        }
        assert!(call(&mut host, count).contains("budget"));
        assert!(call(&mut host, r#"{"capability":"kv.store","op":"get","key":"k"}"#).contains("budget"));
    }

    #[test]
    fn data_and_logs_say_how_to_get_what_they_lack() {
        assert!(call(&mut Host::default(), r#"{"capability":"data.store","op":"count","table":"t"}"#).contains("--manifest"));
        assert!(call(&mut Host::default(), r#"{"capability":"log.read","log":"voice.sessions"}"#).contains("--log-file"));
    }

    #[test]
    fn net_fetch_without_a_fetch_file_says_how_to_get_one() {
        assert!(call(&mut Host::default(), r#"{"capability":"net.fetch","url":"https://a.test/"}"#).contains("--fetch-file"));
    }

    fn leb128(mut n: i64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (n & 0x7f) as u8;
            n >>= 7;
            let done = (n == 0 && byte & 0x40 == 0) || (n == -1 && byte & 0x40 != 0);
            out.push(if done { byte } else { byte | 0x80 });
            if done {
                return out;
            }
        }
    }

    fn section(id: u8, body: &[u8]) -> Vec<u8> {
        assert!(body.len() < 0x80, "a one-byte size is all this builder writes");
        [&[id, body.len() as u8][..], body].concat()
    }

    /// A module whose `run` makes one `host_call(0, len)` and returns what that answers.
    fn module_calling_host_with(len: i32) -> Vec<u8> {
        let mut run = vec![0x00, 0x41, 0x00, 0x41];
        run.extend(leb128(len.into()));
        run.extend([0x10, 0x00, 0x0b]);
        let alloc = [0x04, 0x00, 0x41, 0x00, 0x0b];
        let code = [&[0x02][..], &alloc, &[run.len() as u8], &run].concat();
        let exports = b"\x03\x06memory\x02\x00\x05alloc\x00\x01\x03run\x00\x02";
        [
            &b"\0asm\x01\0\0\0"[..],
            &section(1, &[0x02, 0x60, 0x01, 0x7f, 0x01, 0x7f, 0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7e]),
            &section(2, b"\x01\x04slim\x09host_call\x00\x01"),
            &section(3, &[0x02, 0x00, 0x01]),
            &section(5, &[0x01, 0x00, 0x04]),
            &section(7, exports),
            &section(10, &code),
        ]
        .concat()
    }

    fn answer_to_a_request_of(len: i32) -> String {
        crate::run_bytes(&module_calling_host_with(len), &crate::cli::Options::command("m.wasm", "go", "")).unwrap()
    }

    #[test]
    fn a_request_over_slims_cap_is_answered_with_nothing() {
        assert!(answer_to_a_request_of(64).contains("capability not available"));
        assert!(answer_to_a_request_of(MAX_HOST_CALL_REQUEST_BYTES as i32).contains("capability not available"));
        assert_eq!(answer_to_a_request_of(MAX_HOST_CALL_REQUEST_BYTES as i32 + 1), "");
        assert_eq!(answer_to_a_request_of(i32::MAX), "");
        assert_eq!(answer_to_a_request_of(-1), "");
    }

    #[test]
    fn the_store_survives_a_dump_and_load() {
        let kv = BTreeMap::from([("sheet:a".to_owned(), "{\"n\":\"Aria\"}".to_owned())]);
        assert_eq!(load(&dump(&kv)), kv);
    }
}
