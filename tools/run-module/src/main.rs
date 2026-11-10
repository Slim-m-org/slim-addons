//! Runs a module the way slim runs it, and prints the JSON it answers with.
//!
//! Usage: run-module [options] <module.wasm> <command> [input]
//!        run-module [options] --tick <command> <module.wasm>
//!        run-module [options] --event <name> <data-json> <module.wasm>
//!        run-module [options] --slot <kind> <name> <user,ids> <module.wasm>
//!
//! This exists because `scripts/check-scene-ops.py` needs something to drive a
//! module, and because a module author otherwise has no way to see their own
//! output short of installing it into a space. It implements slim's module ABI
//! v1 and nothing else:
//!
//! - the module exports `memory`, `alloc(len) -> ptr` and
//!   `run(in_ptr, in_len) -> packed(out_ptr << 32 | out_len)`
//! - the module imports nothing, or exactly `slim.host_call`; this refuses
//!   anything else, as slim does - catching an accidental wasi or wasm-bindgen
//!   dependency here rather than at install time
//!
//! `host_call` serves `kv.store`, `net.fetch`, `message.post`, `data.store` and
//! `log.read`, always approved (see `host.rs`). The kv store is in memory, and
//! `--kv-file <path>` keeps it in that file between runs. `net.fetch` answers
//! from the canned responses in `--fetch-file <path>` (see `net.rs`) and refuses
//! any host that file does not name. `message.post` is printed to stderr rather
//! than sent. `data.store` holds the tables the manifest declares (`data.rs`);
//! `--data-file <path>` keeps them between runs. `log.read` serves the logs in
//! `--log-file <path>` (`logs.rs`).
//!
//! The manifest is read from beside the wasm (`../manifest.json` of a
//! `modules/<id>/<version>/module.wasm`), or from `--manifest <path>`. It supplies
//! the data tables, the `settings` every run carries (defaults, with `--setting
//! key=value` laid over them), and what `--event` and `--slot` are checked against.
//!
//! `--event <name> <data-json>` runs `@event` the way slim delivers a subscribed
//! event, as the module's own principal. `--slot <kind> <name> <ids>` runs `@slot`
//! as the viewer `--caller` names, with `--bot <id>` (repeatable) marking which of
//! the asked-about members are bot accounts, and checks the answer against the
//! `slot/1` contract (`slot.rs`).
//!
//! `--now-ms <n>` sets the request's `now_ms`, the clock a module otherwise has
//! no way to read; unset, it is this machine's clock. `--tick <command>` runs a
//! scheduled command the way slim's timer does: empty input, as the module's
//! own principal, with no one typing it.
//!
//! `--caller` puts `{"caller":{"id":...}}` in the request, which is how a
//! seat-based module learns who is acting. `--fuel` meters the run and fails it
//! with `out of fuel` when the budget is spent, the way slim refuses a module
//! over `runtime.limits.fuel`, and prints the fuel the run used to stderr;
//! without it the run is unmetered.
//!
//! Memory and wall-clock limits are not enforced here. The manifest's
//! `runtime.limits` is the authority on those, and the scripts that drive this
//! tool put a timeout around each run.

mod cli;
mod data;
#[cfg(test)]
mod data_tests;
mod events;
mod host;
mod logs;
mod net;
mod settings;
mod slot;

use std::process::ExitCode;

use serde_json::{json, Value};
use wasmi::{Config, Engine, Extern, Instance, Linker, Memory, Module, Store};

use cli::{parse_args, Mode, Options};
use host::Host;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse_args(&args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };

    match run(&options) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run(options: &Options) -> Result<String, String> {
    let path = &options.path;
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    run_bytes(&bytes, options)
}

fn run_bytes(bytes: &[u8], options: &Options) -> Result<String, String> {
    let mut config = Config::default();
    config.consume_fuel(options.fuel.is_some());
    let engine = Engine::new(&config);
    let module = Module::new(&engine, bytes).map_err(|e| format!("not valid wasm: {e}"))?;

    // A module may import nothing, or exactly slim.host_call and nothing else.
    let wanted: Vec<String> = module
        .imports()
        .map(|i| format!("{}::{}", i.module(), i.name()))
        .filter(|name| name != "slim::host_call")
        .collect();
    if !wanted.is_empty() {
        return Err(format!(
            "module imports {}, but slim only instantiates modules that import nothing or just slim.host_call",
            wanted.join(", ")
        ));
    }

    let manifest = load_manifest(options)?;
    let mut store = Store::new(&engine, build_host(options, manifest.as_ref())?);
    if let Some(fuel) = options.fuel {
        store.set_fuel(fuel).map_err(|e| format!("cannot meter the run: {e}"))?;
    }
    let mut linker = Linker::<Host>::new(&engine);
    host::link(&mut linker)?;
    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .map_err(|e| format!("cannot instantiate: {e}"))?;

    let now_ms = options.now_ms.unwrap_or_else(clock_ms);
    let (command, input) = run_input(options, manifest.as_ref(), now_ms)?;
    let given = settings::resolve(manifest.as_ref(), &options.settings)?;
    let request = serde_request(&command, &input, options.caller.as_deref(), (now_ms, given));
    let reply = call_run(&mut store, &instance, request.as_bytes())?;
    if let Some(fuel) = options.fuel {
        eprintln!("fuel used: {}", fuel.saturating_sub(store.get_fuel().unwrap_or(fuel)));
    }
    for (n, content) in store.data().posts.iter().enumerate() {
        eprintln!("message.post #{}: {content}", n + 1);
    }
    save_state(options, store.data())?;
    if let Mode::Slot([_, _, ids]) = &options.mode {
        let problems = slot::check(&reply, &slot::user_ids(ids));
        if !problems.is_empty() {
            return Err(format!("{reply}\nthe slot answer breaks the slot/1 contract:\n  {}", problems.join("\n  ")));
        }
    }
    Ok(reply)
}

/// The command and input the request carries; events and slots use slim's reserved names.
fn run_input(options: &Options, manifest: Option<&Value>, now_ms: i64) -> Result<(String, String), String> {
    match &options.mode {
        Mode::Command { command, input } => Ok((command.clone(), input.clone())),
        Mode::Tick(command) => Ok((command.clone(), String::new())),
        Mode::Event { name, data } => Ok(("@event".to_owned(), events::input(name, data, now_ms, manifest)?)),
        Mode::Slot(slot) => {
            let viewer = options.caller.as_deref().unwrap_or("viewer");
            Ok(("@slot".to_owned(), slot::input(slot, viewer, &options.bots, manifest)?))
        }
    }
}

/// `--manifest`, else the manifest beside a `modules/<id>/<version>/module.wasm`.
fn load_manifest(options: &Options) -> Result<Option<Value>, String> {
    let beside = std::path::Path::new(&options.path).parent().and_then(|v| v.parent()).map(|m| m.join("manifest.json"));
    let (path, explicit) = match (&options.manifest, beside) {
        (Some(path), _) => (std::path::PathBuf::from(path), true),
        (None, Some(path)) if path.is_file() => (path, false),
        _ => return Ok(None),
    };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    match serde_json::from_str(&text) {
        Ok(manifest) => Ok(Some(manifest)),
        Err(e) if explicit => Err(format!("{} is not JSON: {e}", path.display())),
        Err(_) => Ok(None),
    }
}

fn build_host(options: &Options, manifest: Option<&Value>) -> Result<Host, String> {
    let read = |path: &str| std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"));
    let mut host = Host::default();
    if let Some(path) = &options.kv_file {
        host.kv = host::load(&std::fs::read_to_string(path).unwrap_or_default());
    }
    if let Some(path) = &options.fetch_file {
        host.fetch = Some(net::Canned::parse(&read(path)?)?);
    }
    if let Some(path) = &options.log_file {
        host.logs = Some(logs::Logs::parse(&read(path)?)?);
    }
    host.data = manifest.and_then(data::Data::from_manifest);
    if let (Some(data), Some(path)) = (host.data.as_mut(), &options.data_file) {
        data.load(&std::fs::read_to_string(path).unwrap_or_default());
    }
    Ok(host)
}

fn save_state(options: &Options, host: &Host) -> Result<(), String> {
    let write = |path: &str, text: String| std::fs::write(path, text).map_err(|e| format!("cannot write {path}: {e}"));
    if let Some(path) = &options.kv_file {
        write(path, host::dump(&host.kv))?;
    }
    match (&host.data, &options.data_file) {
        (Some(data), Some(path)) => write(path, data.dump()),
        _ => Ok(()),
    }
}

/// This machine's clock in the unit slim's `now_ms` uses: UTC epoch milliseconds.
fn clock_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// The request shape slim sends: `{"command":..., "input":..., "caller":{"id":...}, "entropy":..., "now_ms":..., "settings":{...}}`.
///
/// slim gives every run a fresh `entropy`; set `RUN_MODULE_ENTROPY` to send one,
/// and leave it unset to send none, as a host older than the field would.
fn serde_request(command: &str, input: &str, caller: Option<&str>, (now_ms, settings): (i64, Option<Value>)) -> String {
    let mut request = json!({"command": command, "input": input, "now_ms": now_ms});
    if let Some(id) = caller {
        request["caller"] = json!({"id": id});
    }
    if let Ok(entropy) = std::env::var("RUN_MODULE_ENTROPY") {
        request["entropy"] = json!(entropy);
    }
    if let Some(settings) = settings {
        request["settings"] = settings;
    }
    request.to_string()
}

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn call_run(
    store: &mut Store<Host>,
    instance: &Instance,
    request: &[u8],
) -> Result<String, String> {
    let Some(Extern::Memory(memory)) = instance.get_export(&*store, "memory") else {
        return Err("module exports no memory".to_owned());
    };
    let alloc = instance
        .get_typed_func::<i32, i32>(&*store, "alloc")
        .map_err(|e| format!("module exports no usable alloc: {e}"))?;
    let run = instance
        .get_typed_func::<(i32, i32), i64>(&*store, "run")
        .map_err(|e| format!("module exports no usable run: {e}"))?;

    let in_len = request.len() as i32;
    let in_ptr = alloc
        .call(&mut *store, in_len)
        .map_err(|e| format!("alloc trapped: {e}"))?;
    memory
        .write(&mut *store, in_ptr as usize, request)
        .map_err(|e| format!("alloc returned memory that cannot hold the request: {e}"))?;

    let packed = run
        .call(&mut *store, (in_ptr, in_len))
        .map_err(|e| match e.as_trap_code() {
            Some(wasmi::TrapCode::OutOfFuel) => "out of fuel: the module spent its budget".to_owned(),
            _ => format!("run trapped: {e}"),
        })?;
    let out_ptr = (packed >> 32) as u32 as usize;
    let out_len = (packed & 0xffff_ffff) as u32 as usize;

    read_guest(&memory, store, out_ptr, out_len)
        .ok_or_else(|| "run returned a region outside the module's memory".to_owned())
}

/// A bounds-checked read, for the same reason slim's own is: the pointer and
/// length both come from the module, so a module claiming a huge response gets
/// a refusal rather than this tool trying to allocate it.
fn read_guest(
    memory: &Memory,
    store: &Store<Host>,
    ptr: usize,
    len: usize,
) -> Option<String> {
    let data = memory.data(store);
    let end = ptr.checked_add(len)?;
    let slice = data.get(ptr..end)?;
    Some(String::from_utf8_lossy(slice).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A module whose `run` never returns; alloc hands back offset 0.
    const SPINNER: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // header
        0x01, 0x0c, 0x02, 0x60, 0x01, 0x7f, 0x01, 0x7f, 0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7e, // types
        0x03, 0x03, 0x02, 0x00, 0x01, // functions
        0x05, 0x03, 0x01, 0x00, 0x01, // memory
        0x07, 0x18, 0x03, 0x06, b'm', b'e', b'm', b'o', b'r', b'y', 0x02, 0x00, // exports
        0x05, b'a', b'l', b'l', b'o', b'c', 0x00, 0x00, 0x03, b'r', b'u', b'n', 0x00, 0x01,
        0x0a, 0x10, 0x02, 0x04, 0x00, 0x41, 0x00, 0x0b, // code: alloc
        0x09, 0x00, 0x03, 0x40, 0x0c, 0x00, 0x0b, 0x42, 0x00, 0x0b, // code: run loops forever
    ];

    #[test]
    fn a_caller_and_settings_are_put_in_the_request() {
        let request: Value = serde_json::from_str(&serde_request("play", "", Some("al\"ice"), (7, Some(json!({"rate": 5}))))).unwrap();
        assert_eq!(request, json!({"command": "play", "input": "", "caller": {"id": "al\"ice"}, "now_ms": 7, "settings": {"rate": 5}}));
        let bare = serde_request("play", "", None, (7, None));
        assert!(!bare.contains("caller") && !bare.contains("settings"));
    }

    #[test]
    fn the_request_carries_a_clock() {
        assert!(serde_request("c", "", None, (1_700_000_000_000, None)).contains(r#""now_ms":1700000000000"#));
        assert!(clock_ms() > 1_700_000_000_000);
    }

    #[test]
    fn a_module_that_loops_forever_runs_out_of_fuel() {
        let options = Options { fuel: Some(100_000), ..Options::command("spin.wasm", "go", "") };
        let err = run_bytes(SPINNER, &options).unwrap_err();
        assert!(err.starts_with("out of fuel"), "{err}");
    }
}
