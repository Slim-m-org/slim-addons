//! The commands: `event` (the app and `/event`), `events` (managers) and `tick` (the reminder timer and the daily sweep).

use serde::Deserialize;
use serde_json::{json, Value};

use crate::event::{clean, who, Event, GOING, MAX_TITLE, MAX_WHERE, MAYBE, NO};
use crate::host::Host;
use crate::scene::{self, CANCEL};
use crate::store;
use crate::time::{countdown, format_local, parse_when};

const MAX_TIMER_MS: i64 = 7 * 24 * 3_600_000;
const MIN_TIMER_MS: i64 = 10_000;
/// A timer is allowed to fire this much before its time without re-arming.
const EARLY_SLACK_MS: i64 = 30_000;
const REMINDER_NAMES: usize = 8;

pub struct Call<'a> {
    pub command: &'a str,
    pub input: &'a str,
    pub caller: &'a str,
    pub entropy: &'a str,
    pub now_ms: i64,
    pub channel: &'a str,
}

pub struct Reply {
    pub output: String,
    pub run_after: Option<Value>,
}

/// `Err` is the person-facing reason the command could not do what was asked.
pub type Outcome = Result<Reply, String>;

#[derive(Deserialize)]
struct Action {
    action: String,
    #[serde(default)]
    state: String,
}

pub fn run(host: &dyn Host, call: &Call) -> Outcome {
    match call.command {
        "event" => event(host, call),
        "event-admin" => manage(host, call),
        "tick" => tick(host, call),
        other => Err(format!("unknown command: {other}")),
    }
}

fn plain(output: String) -> Outcome {
    Ok(Reply { output, run_after: None })
}

fn with_timer(output: String, id: &str, event: &Event, now_ms: i64) -> Outcome {
    Ok(Reply { output, run_after: Some(timer(id, event, now_ms)) })
}

/// Arms the reminder for this event, replacing any earlier one, or cancels it when there is none to send.
fn timer(id: &str, event: &Event, now_ms: i64) -> Value {
    if event.x || event.r || event.rem == 0 {
        return json!({"command": "tick", "key": id, "cancel": true});
    }
    let after = (event.rem - now_ms).clamp(MIN_TIMER_MS, MAX_TIMER_MS);
    json!({"command": "tick", "key": id, "input": id, "after_ms": after})
}

fn event(host: &dyn Host, call: &Call) -> Outcome {
    let input = call.input.trim();
    if input.is_empty() {
        return plain(scene::form(None));
    }
    match serde_json::from_str::<Action>(input) {
        Ok(action) => match action.action.strip_prefix("plan:") {
            Some(text) => create(host, call, text),
            None => act(host, call, &action),
        },
        Err(_) => create(host, call, input),
    }
}

fn create(host: &dyn Host, call: &Call, text: &str) -> Outcome {
    if call.caller.is_empty() {
        return Err("slim did not say who you are, so the event cannot have a host".to_owned());
    }
    let mut event = match parse_plan(text, call) {
        Ok(event) => event,
        Err(note) => return plain(scene::form(Some(&note))),
    };
    event.rem = event.remind_at(call.now_ms).unwrap_or(0);
    event.answer(call.caller, GOING).ok();
    let id = new_id(call);
    let (id, event) = store::create(host, &id, event, call.now_ms)?;
    with_timer(scene::event(&event, &id, call.now_ms, None), &id, &event, call.now_ms)
}

fn new_id(call: &Call) -> String {
    let id: String = call.entropy.chars().filter(char::is_ascii_alphanumeric).take(10).collect();
    if id.len() >= 6 { id } else { format!("{:x}", call.now_ms) }
}

/// `title | when | where`, the last part optional.
fn parse_plan(text: &str, call: &Call) -> Result<Event, String> {
    let parts: Vec<&str> = text.split('|').map(str::trim).collect();
    if !(2..=3).contains(&parts.len()) {
        return Err(scene::FORM_USAGE.to_owned());
    }
    let title = clean(parts[0], MAX_TITLE);
    if title.is_empty() {
        return Err("the event needs a title before the first |".to_owned());
    }
    let (at, z) = parse_when(parts[1], call.now_ms)?;
    Ok(Event {
        n: 0, t: title, at, z, w: clean(parts.get(2).copied().unwrap_or(""), MAX_WHERE),
        by: who(call.caller), ch: clean(call.channel, 40), x: false, r: false, rem: 0, rs: Vec::new(),
    })
}

fn act(host: &dyn Host, call: &Call, action: &Action) -> Outcome {
    let id = action.state.as_str();
    let Some(mut event) = store::load(host, id)? else {
        return plain(scene::gone());
    };
    let now = call.now_ms;
    let mut timer_changed = false;
    match action.action.as_str() {
        label @ ("Going" | "Maybe" | "Can't") => answer(&mut event, call, label)?,
        CANCEL => {
            require_host(&event, call, "cancel")?;
            event.x = true;
            timer_changed = true;
        }
        other => {
            if let Some(name) = other.strip_prefix("name:") {
                if !event.set_name(call.caller, name) {
                    return Err("answer Going, Maybe or Can't first, then set your name".to_owned());
                }
            } else if let Some(text) = other.strip_prefix("edit:") {
                require_host(&event, call, "edit")?;
                apply_edit(&mut event, text, now)?;
                timer_changed = true;
            } else {
                return plain(scene::event(&event, id, now, None));
            }
        }
    }
    store::save(host, id, &event)?;
    let frame = scene::event(&event, id, now, None);
    if timer_changed { with_timer(frame, id, &event, now) } else { plain(frame) }
}

fn answer(event: &mut Event, call: &Call, label: &str) -> Result<(), String> {
    if event.x {
        return Err("this event was cancelled".to_owned());
    }
    if event.started(call.now_ms) {
        return Err("this event has started, so answers are closed".to_owned());
    }
    if call.caller.is_empty() {
        return Err("slim did not say who you are, so the answer cannot be kept".to_owned());
    }
    let status = match label {
        "Going" => GOING,
        "Maybe" => MAYBE,
        _ => NO,
    };
    event.answer(call.caller, status).map_err(|_| "this event already has as many answers as it can keep".to_owned())
}

fn require_host(event: &Event, call: &Call, verb: &str) -> Result<(), String> {
    if event.is_host(call.caller) {
        Ok(())
    } else {
        Err(format!("only the host can {verb} this event"))
    }
}

/// Applies `title | when | where` where a blank part keeps what was there.
fn apply_edit(event: &mut Event, text: &str, now_ms: i64) -> Result<(), String> {
    if event.x {
        return Err("this event was cancelled".to_owned());
    }
    if event.started(now_ms) {
        return Err("this event has started and cannot be changed".to_owned());
    }
    let parts: Vec<&str> = text.split('|').map(str::trim).collect();
    if parts.len() > 3 || parts.iter().all(|p| p.is_empty()) {
        return Err(format!("edit as {}; leave a part blank to keep it", scene::FORM_USAGE));
    }
    let title = clean(parts[0], MAX_TITLE);
    let moved = match parts.get(1).filter(|p| !p.is_empty()) {
        Some(when) => Some(parse_when(when, now_ms)?),
        None => None,
    };
    if !title.is_empty() {
        event.t = title;
    }
    if let Some(place) = parts.get(2).filter(|p| !p.is_empty()) {
        event.w = clean(place, MAX_WHERE);
    }
    if let Some((at, z)) = moved {
        event.at = at;
        event.z = z;
        event.r = false;
    }
    event.rem = event.remind_at(now_ms).unwrap_or(0);
    Ok(())
}

fn manage(host: &dyn Host, call: &Call) -> Outcome {
    let input = call.input.trim();
    let (verb, rest) = input.split_once(char::is_whitespace).map_or((input, ""), |(v, r)| (v, r.trim()));
    match verb {
        "" | "list" => plain(list(host, call)?),
        "cancel" | "edit" => {
            let (number, tail) = rest.split_once(char::is_whitespace).map_or((rest, ""), |(n, t)| (n, t.trim()));
            let (id, mut event) = find(host, call, number)?;
            if verb == "edit" {
                apply_edit(&mut event, tail, call.now_ms)?;
            } else if event.x {
                return Err(format!("#{} is already cancelled", event.n));
            } else {
                event.x = true;
            }
            store::save(host, &id, &event)?;
            let done = if verb == "edit" { "Updated" } else { "Cancelled" };
            Ok(Reply { output: format!("{done} #{}: {}. Its message updates the next time someone opens it.", event.n, event.t), run_after: Some(timer(&id, &event, call.now_ms)) })
        }
        _ => Err("use: /event-admin, /event-admin cancel <number>, or /event-admin edit <number> title | when | where".to_owned()),
    }
}

fn here<'a>(call: &Call, events: &'a [(String, Event)]) -> Vec<&'a (String, Event)> {
    events.iter().filter(|(_, e)| call.channel.is_empty() || e.ch.is_empty() || e.ch == clean(call.channel, 40)).collect()
}

fn list(host: &dyn Host, call: &Call) -> Result<String, String> {
    let events = store::all(host)?;
    let lines: Vec<String> = here(call, &events)
        .iter()
        .map(|(_, e)| {
            let state = if e.x { "cancelled".to_owned() } else { countdown(e.at - call.now_ms) };
            format!("#{} {} - {} - {} - {} going", e.n, e.t, format_local(e.at, e.z), state, e.counts()[usize::from(GOING)])
        })
        .collect();
    Ok(if lines.is_empty() { "No events are planned here.".to_owned() } else { lines.join("\n") })
}

fn find(host: &dyn Host, call: &Call, number: &str) -> Result<(String, Event), String> {
    let wanted: u32 = number.trim_start_matches('#').parse().map_err(|_| "name the event by its number, like /event-admin cancel 3".to_owned())?;
    let events = store::all(host)?;
    let mut matches = here(call, &events).into_iter().filter(|(_, e)| e.n == wanted);
    match (matches.next(), matches.next()) {
        (Some((id, event)), None) => Ok((id.clone(), event.clone())),
        (None, _) => Err(format!("no event #{wanted} here")),
        _ => Err(format!("more than one event is numbered {wanted}; ask the host to cancel theirs")),
    }
}

fn tick(host: &dyn Host, call: &Call) -> Outcome {
    let id = call.input.trim();
    if id.is_empty() {
        let removed = store::prune(host, call.now_ms)?;
        return plain(format!("Removed {removed} finished events."));
    }
    let Some(mut event) = store::load(host, id)? else {
        return plain("No such event.".to_owned());
    };
    if event.x || event.r || event.rem == 0 || call.now_ms >= event.at {
        return plain("Nothing to remind.".to_owned());
    }
    if call.now_ms + EARLY_SLACK_MS < event.rem {
        return with_timer("Not yet.".to_owned(), id, &event, call.now_ms);
    }
    host.post(&reminder_text(&event, call.now_ms)).map_err(|e| format!("could not post the reminder: {e}"))?;
    event.r = true;
    store::save(host, id, &event)?;
    plain("Posted the reminder.".to_owned())
}

fn reminder_text(event: &Event, now_ms: i64) -> String {
    let place = if event.w.is_empty() { String::new() } else { format!(" at {}", event.w) };
    let going = event.names(GOING);
    let who = match going.len() {
        0 => "Nobody has said they are going yet.".to_owned(),
        n => {
            let shown: Vec<String> = going.iter().take(REMINDER_NAMES).cloned().collect();
            let more = if n > shown.len() { format!(" and {} more", n - shown.len()) } else { String::new() };
            format!("{n} going: {}{more}.", shown.join(", "))
        }
    };
    format!("Reminder: {} (#{}) {}{place}, {}. {who}", event.t, event.n, countdown(event.at - now_ms), format_local(event.at, event.z))
}
