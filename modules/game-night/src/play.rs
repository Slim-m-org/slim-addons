//! The `night` command: the form, planning a night, and a member's taps on it.

use serde::Deserialize;

use crate::host::Host;
use crate::night::{Answer, Night, Store, CANCELLED, OPEN};
use crate::parse;
use crate::scene::{self, CANCEL, CLOSE, REFRESH};
use crate::settings::Settings;

const MAX_ID: usize = 64;
const GONE: &str = "this game night is gone";
const ANONYMOUS: &str = "this server did not say who you are";

pub struct Run {
    pub caller: String,
    pub now_ms: i64,
    entropy: String,
}

impl Run {
    pub fn of(caller: &str, now_ms: i64, entropy: &str) -> Run {
        // The id is a key part joined with `|`, so it keeps to characters that cannot break it.
        let caller = caller.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(MAX_ID).collect();
        Run { caller, now_ms, entropy: entropy.to_owned() }
    }

    fn new_id(&self) -> String {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let seed = format!("{}|{}|{}", self.entropy, self.caller, self.now_ms);
        for byte in seed.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
        format!("{hash:016x}")
    }
}

#[derive(Deserialize)]
struct Action {
    action: String,
    #[serde(default)]
    state: String,
}

pub fn night<H: Host>(host: &H, settings: &Settings, run: &Run, input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return scene::form(None);
    }
    match serde_json::from_str::<Action>(trimmed) {
        Ok(action) => act(host, settings, run, &action),
        // A slash-command launch hands the typed text straight through, so read it as a plan.
        Err(_) => plan(host, settings, run, trimmed),
    }
}

fn act<H: Host>(host: &H, settings: &Settings, run: &Run, action: &Action) -> String {
    if let Some(text) = action.action.strip_prefix("plan:") {
        return plan(host, settings, run, text);
    }
    if action.state.is_empty() {
        return scene::form(None);
    }
    let store = Store::new(host);
    let loaded = match store.load(&action.state) {
        Ok(Some(night)) => night,
        Ok(None) => return scene::form(Some(GONE)),
        Err(why) => return scene::form(Some(&format!("could not load the night ({why})"))),
    };
    let note = tap(&store, settings, run, &loaded, &action.action);
    // The tap may have changed the night, so what is shown is read again.
    let current = store.load(&loaded.id).ok().flatten().unwrap_or(loaded);
    draw(&store, &current, run, note.err().as_deref())
}

fn draw<H: Host>(store: &Store<H>, night: &Night, run: &Run, note: Option<&str>) -> String {
    match store.tally(night, &run.caller) {
        Ok(tally) => scene::night(night, &tally, run.now_ms, note),
        Err(why) => scene::form(Some(&format!("could not read the votes ({why})"))),
    }
}

/// Applies one tap; `Err` carries the sentence to show instead of the usual status.
fn tap<H: Host>(store: &Store<H>, settings: &Settings, run: &Run, night: &Night, action: &str) -> Result<(), String> {
    if run.caller.is_empty() && action != REFRESH {
        return Err(ANONYMOUS.to_owned());
    }
    if let Some(index) = action.strip_prefix("vote:") {
        return vote(store, run, night, index);
    }
    if let Some(answer) = Answer::parse(action) {
        return reply(store, run, night, answer);
    }
    match action {
        CLOSE => close(store, settings, run, night),
        CANCEL => cancel(store, run, night),
        _ => Ok(()),
    }
}

fn vote<H: Host>(store: &Store<H>, run: &Run, night: &Night, index: &str) -> Result<(), String> {
    let Some(slot) = index.parse::<usize>().ok().filter(|i| *i < night.slots.len()) else { return Ok(()) };
    if night.chosen != OPEN {
        return Err("the time is settled, so only your answer can change".to_owned());
    }
    store.toggle_slot(&night.id, &run.caller, slot)
}

fn reply<H: Host>(store: &Store<H>, run: &Run, night: &Night, answer: Answer) -> Result<(), String> {
    let started = night.chosen >= 0 && night.slots.get(night.chosen as usize).is_some_and(|s| s.start_ms <= run.now_ms);
    if night.chosen == CANCELLED || started {
        return Err("this game night is over".to_owned());
    }
    store.answer(&night.id, &run.caller, answer)
}

fn close<H: Host>(store: &Store<H>, settings: &Settings, run: &Run, night: &Night) -> Result<(), String> {
    if night.host != run.caller {
        return Err("only the host can close the poll".to_owned());
    }
    if night.chosen != OPEN {
        return Ok(());
    }
    let tally = store.tally(night, "")?;
    let top = tally.per_slot.iter().copied().max().unwrap_or(0);
    if top == 0 {
        return Err("nobody has picked a time yet".to_owned());
    }
    let chosen = tally.per_slot.iter().position(|v| *v == top).unwrap_or(0);
    let remind_at = (settings.remind_min > 0).then(|| night.slots[chosen].start_ms - settings.remind_min * 60_000);
    store.close(&night.id, chosen, remind_at).map(|_| ())
}

fn cancel<H: Host>(store: &Store<H>, run: &Run, night: &Night) -> Result<(), String> {
    if night.host != run.caller {
        return Err("only the host can cancel it".to_owned());
    }
    store.cancel(&night.id).map(|_| ())
}

fn plan<H: Host>(host: &H, settings: &Settings, run: &Run, text: &str) -> String {
    if run.caller.is_empty() {
        return scene::form(Some(ANONYMOUS));
    }
    let plan = match parse::plan(text, run.now_ms, settings.utc_offset_min) {
        Ok(plan) => plan,
        Err(why) => return scene::form(Some(&why)),
    };
    let created = Night {
        id: run.new_id(), game: plan.game, role: plan.role, host: run.caller.clone(), slots: plan.slots,
        chosen: OPEN, remind_at: 0, created: run.now_ms,
    };
    let store = Store::new(host);
    if let Err(why) = store.create(&created) {
        return scene::form(Some(&format!("could not save the night ({why})")));
    }
    let note = ping(host, &created).err().map(|why| format!("planned, but the ping did not go out ({why})"));
    draw(&store, &created, run, note.as_deref())
}

/// Calls the group in: only a night that names a role pings anyone.
fn ping<H: Host>(host: &H, night: &Night) -> Result<(), String> {
    if night.role.is_empty() {
        return Ok(());
    }
    let times: Vec<&str> = night.slots.iter().map(|s| s.label.as_str()).collect();
    let content = format!("@[{}] {} - who is up for it? Tap the times that work: {}", night.role, night.game, times.join(", "));
    crate::remind::post(host, &content)
}
