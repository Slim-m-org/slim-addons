//! Runs a module the way slim runs it, and prints the JSON it answers with.
//!
//! Usage: run-module [--caller <id>] [--fuel <n>] [--kv-file <path>]
//!                   [--fetch-file <path>] [--now-ms <n>] <module.wasm> <command> [input]
//!        run-module [options] --tick <command> <module.wasm>
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
//! `host_call` serves `kv.store`, `net.fetch` and `message.post`, always
//! approved (see `host.rs`). The store is in memory, and `--kv-file <path>`
//! keeps it in that file between runs. `net.fetch` answers from the canned
//! responses in `--fetch-file <path>` (see `net.rs`) and refuses any host that
//! file does not name. `message.post` is printed to stderr rather than sent.
//!
//! `--now-ms <n>` sets the request's `now_ms`, the clock a module otherwise has
//! no way to read; unset, it is this machine's clock. `--tick <command>` runs a
//! scheduled command the way slim's timer does: empty input, as the module's
//! own principal, with no one typing it.
//!
//! `--caller` puts `{"caller":{"id":...}}` in the request, which is how a
//! seat-based module learns who is acting. `--fuel` meters the run and fails it
//! with `out of fuel` when the budget is spent, the way slim refuses a module
//! over `runtime.limits.fuel`; without it the run is unmetered.
//!
//! Memory and wall-clock limits are not enforced here. The manifest's
//! `runtime.limits` is the authority on those, and the scripts that drive this
//! tool put a timeout around each run.

mod host;
mod net;

use std::process::ExitCode;

use wasmi::{Config, Engine, Extern, Instance, Linker, Memory, Module, Store};

use host::Host;

struct Options {
    path: String,
    command: String,
    input: String,
    caller: Option<String>,
    fuel: Option<u64>,
    kv_file: Option<String>,
    fetch_file: Option<String>,
    now_ms: Option<i64>,
    tick: bool,
}

/// The caller id a scheduled run carries: a module's own principal, not a person.
const SCHEDULED_CALLER: &str = "scheduled";

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut positional = Vec::new();
    let mut caller = None;
    let mut fuel = None;
    let mut kv_file = None;
    let mut fetch_file = None;
    let mut now_ms = None;
    let mut tick = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--caller" => caller = Some(iter.next().ok_or("--caller needs an id")?.clone()),
            "--fuel" => {
                let n = iter.next().ok_or("--fuel needs a number")?;
                fuel = Some(n.parse::<u64>().map_err(|_| format!("--fuel: {n} is not a number"))?);
            }
            "--kv-file" => kv_file = Some(iter.next().ok_or("--kv-file needs a path")?.clone()),
            "--fetch-file" => fetch_file = Some(iter.next().ok_or("--fetch-file needs a path")?.clone()),
            "--now-ms" => {
                let n = iter.next().ok_or("--now-ms needs a number")?;
                now_ms = Some(n.parse::<i64>().map_err(|_| format!("--now-ms: {n} is not a number"))?);
            }
            "--tick" => tick = Some(iter.next().ok_or("--tick needs a command")?.clone()),
            _ => positional.push(arg.clone()),
        }
    }
    let usage = "usage: run-module [--caller <id>] [--fuel <n>] [--kv-file <path>] [--fetch-file <path>] [--now-ms <n>] \
                 (<module.wasm> <command> [input] | --tick <command> <module.wasm>)";
    let ticking = tick.is_some();
    let mut options = match (tick, positional.as_slice()) {
        (Some(command), [path]) => Options::new(path, &command, "", caller.or_else(|| Some(SCHEDULED_CALLER.to_owned())), fuel),
        (None, [path, command]) => Options::new(path, command, "", caller, fuel),
        (None, [path, command, input]) => Options::new(path, command, input, caller, fuel),
        _ => return Err(usage.to_owned()),
    };
    options.kv_file = kv_file;
    options.fetch_file = fetch_file;
    options.now_ms = now_ms;
    options.tick = ticking;
    Ok(options)
}

impl Options {
    fn new(path: &str, command: &str, input: &str, caller: Option<String>, fuel: Option<u64>) -> Self {
        Options {
            path: path.into(),
            command: command.into(),
            input: input.into(),
            caller,
            fuel,
            kv_file: None,
            fetch_file: None,
            now_ms: None,
            tick: false,
        }
    }
}

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

    let mut store = Store::new(&engine, Host::default());
    if let Some(path) = &options.kv_file {
        store.data_mut().kv = host::load(&std::fs::read_to_string(path).unwrap_or_default());
    }
    if let Some(path) = &options.fetch_file {
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
        store.data_mut().fetch = Some(net::Canned::parse(&text)?);
    }
    if let Some(fuel) = options.fuel {
        store.set_fuel(fuel).map_err(|e| format!("cannot meter the run: {e}"))?;
    }
    let mut linker = Linker::<Host>::new(&engine);
    host::link(&mut linker)?;
    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .map_err(|e| format!("cannot instantiate: {e}"))?;

    let now_ms = options.now_ms.unwrap_or_else(clock_ms);
    let request = serde_request(&options.command, &options.input, options.caller.as_deref(), now_ms);
    let reply = call_run(&mut store, &instance, request.as_bytes());
    for (n, content) in store.data().posts.iter().enumerate() {
        eprintln!("message.post #{}: {content}", n + 1);
    }
    if let Some(path) = &options.kv_file {
        std::fs::write(path, host::dump(&store.data().kv)).map_err(|e| format!("cannot write {path}: {e}"))?;
    }
    reply
}

/// This machine's clock in the unit slim's `now_ms` uses: UTC epoch milliseconds.
fn clock_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// The request shape slim sends: `{"command":..., "input":..., "caller":{"id":...}, "entropy":..., "now_ms":...}`.
///
/// Hand-built rather than pulling in serde, so this tool stays a single small
/// dependency on the same engine slim uses and nothing more. slim gives every
/// run a fresh `entropy`; set `RUN_MODULE_ENTROPY` to send one, and leave it
/// unset to send none, as a host older than the field would.
fn serde_request(command: &str, input: &str, caller: Option<&str>, now_ms: i64) -> String {
    let caller = caller
        .map(|id| format!(",\"caller\":{{\"id\":\"{}\"}}", escape(id)))
        .unwrap_or_default();
    let entropy = std::env::var("RUN_MODULE_ENTROPY")
        .map(|value| format!(",\"entropy\":\"{}\"", escape(&value)))
        .unwrap_or_default();
    format!(
        "{{\"command\":\"{}\",\"input\":\"{}\"{caller}{entropy},\"now_ms\":{now_ms}}}",
        escape(command),
        escape(input)
    )
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

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_caller_is_put_in_the_request() {
        let request = serde_request("play", "", Some("al\"ice"), 7);
        assert_eq!(request, r#"{"command":"play","input":"","caller":{"id":"al\"ice"},"now_ms":7}"#);
        assert!(!serde_request("play", "", None, 7).contains("caller"));
    }

    #[test]
    fn flags_are_read_wherever_they_sit() {
        let o = parse_args(&args(&["--caller", "u1", "m.wasm", "play", "x", "--fuel", "9"])).unwrap();
        assert_eq!((o.caller.as_deref(), o.fuel, o.input.as_str()), (Some("u1"), Some(9), "x"));
        assert!(parse_args(&args(&["--fuel", "many", "m.wasm", "play"])).is_err());
        assert!(parse_args(&args(&["m.wasm"])).is_err());
    }

    #[test]
    fn tick_runs_a_scheduled_command_as_the_modules_own_caller() {
        let o = parse_args(&args(&["--tick", "digest", "--now-ms", "1700000000000", "m.wasm"])).unwrap();
        assert_eq!((o.command.as_str(), o.input.as_str(), o.tick), ("digest", "", true));
        assert_eq!((o.caller.as_deref(), o.now_ms), (Some("scheduled"), Some(1_700_000_000_000)));
        assert!(parse_args(&args(&["--tick", "digest", "m.wasm", "extra", "args"])).is_err());
        assert!(parse_args(&args(&["--tick"])).is_err());
        let o = parse_args(&args(&["--fetch-file", "f.json", "m.wasm", "cmd", "x"])).unwrap();
        assert_eq!(o.fetch_file.as_deref(), Some("f.json"));
    }

    #[test]
    fn the_request_carries_a_clock() {
        assert!(serde_request("c", "", None, 1_700_000_000_000).ends_with(r#","now_ms":1700000000000}"#));
        assert!(clock_ms() > 1_700_000_000_000);
    }

    #[test]
    fn a_module_that_loops_forever_runs_out_of_fuel() {
        let options = Options::new("spin.wasm", "go", "", None, Some(100_000));
        let err = run_bytes(SPINNER, &options).unwrap_err();
        assert!(err.starts_with("out of fuel"), "{err}");
    }
}
