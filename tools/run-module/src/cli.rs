//! The command line: which run to make and what to simulate around it.

/// What the run is: a command, or one of the runs slim makes without a person typing.
#[derive(Debug, PartialEq)]
pub enum Mode {
    Command { command: String, input: String },
    /// A scheduled command: empty input, as the module's own principal.
    Tick(String),
    /// An `@event` delivery: the event name and its `data` JSON.
    Event { name: String, data: String },
    /// An `@slot` ask: kind, name and a comma-separated list of member ids.
    Slot([String; 3]),
}

pub struct Options {
    pub path: String,
    pub mode: Mode,
    pub caller: Option<String>,
    pub fuel: Option<u64>,
    pub kv_file: Option<String>,
    pub fetch_file: Option<String>,
    pub data_file: Option<String>,
    pub log_file: Option<String>,
    pub manifest: Option<String>,
    pub settings: Vec<(String, String)>,
    /// Which of a `--slot` run's user ids are bot accounts.
    pub bots: Vec<String>,
    pub now_ms: Option<i64>,
}

/// The caller id a scheduled or event run carries: a module's own principal, not a person.
pub const SCHEDULED_CALLER: &str = "scheduled";

pub const USAGE: &str = "usage: run-module [--caller <id>] [--fuel <n>] [--kv-file <path>] [--fetch-file <path>] [--data-file <path>]\n\
    \x20                 [--log-file <path>] [--manifest <path>] [--setting <key>=<value>]... [--now-ms <n>]\n\
    \x20                 [--bot <id>]...\n\
    \x20                 (<module.wasm> <command> [input]\n\
    \x20                  | --tick <command> <module.wasm>\n\
    \x20                  | --event <name> <data-json> <module.wasm>\n\
    \x20                  | --slot <kind> <name> <user,ids> <module.wasm>)";

pub fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut positional = Vec::new();
    let mut mode: Option<Mode> = None;
    let mut options = Options::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let mut value = |flag: &str, what: &str| rest.next().cloned().ok_or(format!("{flag} needs {what}"));
        match arg.as_str() {
            "--caller" => options.caller = Some(value(arg, "an id")?),
            "--fuel" => options.fuel = Some(number::<u64>(arg, &value(arg, "a number")?)?),
            "--now-ms" => options.now_ms = Some(number::<i64>(arg, &value(arg, "a number")?)?),
            "--kv-file" => options.kv_file = Some(value(arg, "a path")?),
            "--fetch-file" => options.fetch_file = Some(value(arg, "a path")?),
            "--data-file" => options.data_file = Some(value(arg, "a path")?),
            "--log-file" => options.log_file = Some(value(arg, "a path")?),
            "--manifest" => options.manifest = Some(value(arg, "a path")?),
            "--setting" => {
                let pair = value(arg, "key=value")?;
                let (key, raw) = pair.split_once('=').ok_or(format!("--setting needs key=value, not {pair}"))?;
                options.settings.push((key.to_owned(), raw.to_owned()));
            }
            "--bot" => options.bots.push(value(arg, "an id")?),
            "--tick" => mode = Some(Mode::Tick(value(arg, "a command")?)),
            "--event" => mode = Some(Mode::Event { name: value(arg, "an event name")?, data: value(arg, "the event data")? }),
            "--slot" => mode = Some(Mode::Slot([value(arg, "a kind")?, value(arg, "a name")?, value(arg, "user ids")?])),
            _ => positional.push(arg.clone()),
        }
    }
    match (mode, positional.as_slice()) {
        (Some(mode), [path]) => {
            options.path = path.clone();
            options.mode = mode;
        }
        (None, [path, command]) => {
            options.path = path.clone();
            options.mode = Mode::Command { command: command.clone(), input: String::new() };
        }
        (None, [path, command, input]) => {
            options.path = path.clone();
            options.mode = Mode::Command { command: command.clone(), input: input.clone() };
        }
        _ => return Err(USAGE.to_owned()),
    }
    if !options.bots.is_empty() && !matches!(options.mode, Mode::Slot(_)) {
        return Err("--bot only applies to --slot".to_owned());
    }
    if options.caller.is_none() && matches!(options.mode, Mode::Tick(_) | Mode::Event { .. }) {
        options.caller = Some(SCHEDULED_CALLER.to_owned());
    }
    Ok(options)
}

fn number<T: std::str::FromStr>(flag: &str, raw: &str) -> Result<T, String> {
    raw.parse().map_err(|_| format!("{flag}: {raw} is not a number"))
}

impl Options {
    fn new() -> Self {
        Options {
            path: String::new(),
            mode: Mode::Tick(String::new()),
            caller: None,
            fuel: None,
            kv_file: None,
            fetch_file: None,
            data_file: None,
            log_file: None,
            manifest: None,
            settings: Vec::new(),
            bots: Vec::new(),
            now_ms: None,
        }
    }

    /// A plain command run with nothing simulated around it.
    #[cfg(test)]
    pub fn command(path: &str, command: &str, input: &str) -> Self {
        Options { path: path.into(), mode: Mode::Command { command: command.into(), input: input.into() }, ..Options::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flags_are_read_wherever_they_sit() {
        let o = parse_args(&args(&["--caller", "u1", "m.wasm", "play", "x", "--fuel", "9"])).unwrap();
        let Mode::Command { input, .. } = &o.mode else { panic!("not a command") };
        assert_eq!((o.caller.as_deref(), o.fuel, input.as_str()), (Some("u1"), Some(9), "x"));
        assert!(parse_args(&args(&["--fuel", "many", "m.wasm", "play"])).is_err());
        assert!(parse_args(&args(&["m.wasm"])).is_err());
    }

    #[test]
    fn tick_runs_a_scheduled_command_as_the_modules_own_caller() {
        let o = parse_args(&args(&["--tick", "digest", "--now-ms", "1700000000000", "m.wasm"])).unwrap();
        assert_eq!(o.mode, Mode::Tick("digest".into()));
        assert_eq!((o.caller.as_deref(), o.now_ms), (Some("scheduled"), Some(1_700_000_000_000)));
        assert!(parse_args(&args(&["--tick", "digest", "m.wasm", "extra", "args"])).is_err());
        assert!(parse_args(&args(&["--tick"])).is_err());
        let o = parse_args(&args(&["--fetch-file", "f.json", "m.wasm", "cmd", "x"])).unwrap();
        assert_eq!(o.fetch_file.as_deref(), Some("f.json"));
    }

    #[test]
    fn events_slots_and_settings_are_read() {
        let o = parse_args(&args(&["--event", "member.joined", "{}", "--setting", "a=1", "--setting", "b=x=y", "m.wasm"])).unwrap();
        assert_eq!(o.mode, Mode::Event { name: "member.joined".into(), data: "{}".into() });
        assert_eq!((o.caller.as_deref(), o.settings.len(), o.settings[1].1.as_str()), (Some("scheduled"), 2, "x=y"));
        let o = parse_args(&args(&["--slot", "member.badge", "level", "u1,u2", "--caller", "viewer", "m.wasm"])).unwrap();
        assert_eq!(o.mode, Mode::Slot(["member.badge".into(), "level".into(), "u1,u2".into()]));
        assert_eq!(o.caller.as_deref(), Some("viewer"));
        assert!(parse_args(&args(&["--setting", "novalue", "m.wasm", "c"])).is_err());
    }

    #[test]
    fn bots_mark_slot_members_and_nothing_else() {
        let o = parse_args(&args(&["--slot", "member.badge", "level", "u1,u2", "--bot", "u2", "--bot", "u1", "m.wasm"])).unwrap();
        assert_eq!(o.bots, ["u2", "u1"]);
        assert!(parse_args(&args(&["--bot", "u2", "m.wasm", "play"])).err().is_some_and(|e| e.contains("only applies to --slot")));
        assert!(parse_args(&args(&["--slot", "member.badge", "level", "u1", "m.wasm", "--bot"])).is_err());
    }
}
