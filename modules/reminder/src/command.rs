//! The four commands: set, list and cancel a reminder, and the timer's `due`.

use serde_json::{json, Value};

use crate::host::{Host, KvError, PostError};
use crate::parse::{self, words};
use crate::store::{self, Added, Reminder, MAX_PENDING, MAX_PER_CALLER};

/// Reminders one tick posts in one message; the rest wait for the next tick.
const MAX_PER_POST: usize = 10;
/// The longest a timer the host holds can wait; a reminder due later is armed again when it fires.
const MAX_TIMER_MS: i64 = 7 * 24 * 60 * 60_000;
const MIN_TIMER_MS: i64 = 10_000;
/// A timer fires up to this early without re-arming.
const EARLY_MS: i64 = 15_000;
/// How long past its time a reminder keeps being tried before it is given up.
const GIVE_UP_MS: i64 = 60 * 60_000;
const RETRY_MS: i64 = 5 * 60_000;

pub struct Call<'a> {
    pub command: &'a str,
    pub input: &'a str,
    pub caller: &'a str,
    pub channel: &'a str,
    pub now_ms: i64,
    pub entropy: &'a str,
}

pub struct Reply {
    pub output: String,
    pub run_after: Option<Value>,
}

fn said(output: String) -> Result<Reply, String> {
    Ok(Reply { output, run_after: None })
}

pub fn run<H: Host>(host: &H, call: &Call) -> Result<Reply, String> {
    match call.command {
        "remind" => remind(host, call),
        "list" => list(host, call).and_then(said),
        "cancel" => cancel(host, call),
        "due" if call.input.trim().is_empty() => due(host, call.now_ms).and_then(said),
        "due" => fire(host, call.input.trim(), call.now_ms),
        other => Err(format!("unknown command: {other}")),
    }
}

fn kv(error: KvError) -> String {
    error.text()
}

fn need_clock(now_ms: i64) -> Result<(), String> {
    if now_ms > 0 {
        Ok(())
    } else {
        Err("This server does not give modules the time, so Reminder cannot work here.".to_string())
    }
}

fn need_caller(caller: &str) -> Result<(), String> {
    if caller.is_empty() {
        Err("slim did not say who you are, so Reminder cannot keep your reminders apart.".to_string())
    } else {
        Ok(())
    }
}

/// The host runs `due` once for this reminder, in the channel it was set in.
fn timer(id: &str, after_ms: i64) -> Value {
    json!({"command": "due", "key": id, "input": id, "after_ms": after_ms.clamp(MIN_TIMER_MS, MAX_TIMER_MS)})
}

fn remind<H: Host>(host: &H, call: &Call) -> Result<Reply, String> {
    need_clock(call.now_ms)?;
    need_caller(call.caller)?;
    let parsed = parse::parse(call.input)?;
    let new = store::New {
        due_ms: call.now_ms + parsed.delay_ms,
        salt: call.entropy,
        owner: call.caller,
        channel: call.channel,
        text: &parsed.text,
    };
    match store::add(host, &new).map_err(kv)? {
        Added::Id(id) => Ok(Reply {
            output: format!("Reminder set for {} from now: {}", words(parsed.delay_ms), parsed.text),
            run_after: (!call.channel.is_empty()).then(|| timer(&id, parsed.delay_ms)),
        }),
        Added::OwnerFull => Err(format!("You already have {MAX_PER_CALLER} reminders waiting. Cancel one with /unremind first.")),
        Added::SpaceFull => Err(format!("There are already {MAX_PENDING} reminders waiting in this space. Try again once some have gone out.")),
    }
}

fn list<H: Host>(host: &H, call: &Call) -> Result<String, String> {
    need_caller(call.caller)?;
    let mine = store::of(host, call.caller).map_err(kv)?;
    if mine.is_empty() {
        return Ok("No reminders waiting.".to_string());
    }
    let lines: Vec<String> = mine
        .iter()
        .enumerate()
        .map(|(n, r)| format!("{}. in {}: {}", n + 1, words(r.due_ms - call.now_ms), r.text))
        .collect();
    Ok(lines.join("\n"))
}

fn cancel<H: Host>(host: &H, call: &Call) -> Result<Reply, String> {
    need_caller(call.caller)?;
    let number: usize = call
        .input
        .trim()
        .trim_start_matches('#')
        .parse()
        .map_err(|_| "Give the number from /reminders: /unremind 2".to_string())?;
    let mine = store::of(host, call.caller).map_err(kv)?;
    let chosen = number
        .checked_sub(1)
        .and_then(|n| mine.get(n))
        .ok_or_else(|| format!("There is no reminder {number}. /reminders lists them."))?;
    store::remove(host, std::slice::from_ref(&chosen.id)).map_err(kv)?;
    Ok(Reply {
        output: format!("Cancelled: {}", chosen.text),
        run_after: (!chosen.channel.is_empty()).then(|| json!({"command": "due", "key": chosen.id, "cancel": true})),
    })
}

/// A reminder's own timer, run in the channel it was set in: posts it there once it is due.
/// A post that fails is tried again until the reminder is long overdue, then given up.
fn fire<H: Host>(host: &H, id: &str, now_ms: i64) -> Result<Reply, String> {
    need_clock(now_ms)?;
    let Some(reminder) = store::load(host, id).map_err(kv)? else {
        return said(String::new());
    };
    if reminder.due_ms - now_ms > EARLY_MS {
        return Ok(Reply { output: String::new(), run_after: Some(timer(id, reminder.due_ms - now_ms)) });
    }
    match host.post(&format!("Reminder: {}", reminder.text)) {
        Ok(()) => {
            store::remove(host, std::slice::from_ref(&reminder.id)).map_err(kv)?;
            said("Posted 1 reminder.".to_string())
        }
        Err(error) if now_ms - reminder.due_ms < GIVE_UP_MS => {
            let note = post_failure(error);
            Ok(Reply { output: format!("{note} Trying again."), run_after: Some(timer(id, RETRY_MS)) })
        }
        Err(error) => {
            store::remove(host, std::slice::from_ref(&reminder.id)).map_err(kv)?;
            Err(post_failure(error))
        }
    }
}

/// The schedule: posts what has come due and was set where no channel is known, then forgets it.
/// A reminder is deleted only after its post went out, so a refused post is tried again next tick.
/// A reminder tied to a channel is left to its own timer and dropped only when that never ran.
fn due<H: Host>(host: &H, now_ms: i64) -> Result<String, String> {
    need_clock(now_ms)?;
    let live = store::all(host).map_err(kv)?;
    let (tied, open): (Vec<Reminder>, Vec<Reminder>) = live.into_iter().partition(|r| !r.channel.is_empty());
    let lost = tied.into_iter().filter(|r| now_ms - r.due_ms >= GIVE_UP_MS).map(|r| r.id);
    let mut ready = Vec::new();
    for reminder in open {
        if reminder.due_ms > now_ms {
            break;
        }
        ready.push(reminder);
        if ready.len() == MAX_PER_POST {
            break;
        }
    }
    let mut cleared: Vec<String> = lost.collect();
    if ready.is_empty() {
        store::remove(host, &cleared).map_err(kv)?;
        return Ok(String::new());
    }
    let lines: Vec<String> = ready.iter().map(|r| format!("Reminder: {}", r.text)).collect();
    host.post(&lines.join("\n")).map_err(post_failure)?;
    cleared.extend(ready.iter().map(|r| r.id.clone()));
    store::remove(host, &cleared).map_err(kv)?;
    Ok(format!("Posted {} reminder{}.", ready.len(), if ready.len() == 1 { "" } else { "s" }))
}

fn post_failure(error: PostError) -> String {
    match error {
        PostError::Unavailable => "Reminder needs an admin to allow it to post and pick a channel in the Dock.".to_string(),
        PostError::Refused(reason) => format!("The reminder could not be posted: {reason}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeHost;

    const NOW: i64 = 1_700_000_000_000;

    fn run_reply(host: &FakeHost, who: (&str, &str), command: &str, input: &str, now: i64) -> Result<Reply, String> {
        host.next_run();
        let (caller, channel) = who;
        run(host, &Call { command, input, caller, channel, now_ms: now, entropy: "ab12cd34" })
    }

    fn run_as(host: &FakeHost, who: (&str, &str), command: &str, input: &str, now: i64) -> Result<String, String> {
        run_reply(host, who, command, input, now).map(|r| r.output)
    }

    fn run_at(host: &FakeHost, command: &str, input: &str, now: i64) -> Result<String, String> {
        run_as(host, ("alice", ""), command, input, now)
    }

    /// Sets a reminder as a distinct member, so a space limit can be reached without the per-member one.
    fn fill_space(host: &FakeHost, count: usize, text: &str, now: i64) {
        for n in 0..count {
            let who = format!("member{n}");
            host.next_run();
            let call = Call { command: "remind", input: text, caller: &who, channel: "", now_ms: now + n as i64, entropy: &format!("e{n:03}") };
            run(host, &call).unwrap();
        }
    }

    #[test]
    fn a_reminder_set_now_is_listed_then_posted_once_due_and_forgotten() {
        let host = FakeHost::default();
        assert!(run_at(&host, "remind", "30m take the bread out", NOW).unwrap().contains("30 minutes"));
        assert_eq!(run_at(&host, "list", "", NOW + 60_000).unwrap(), "1. in 29 minutes: take the bread out");

        assert_eq!(run_at(&host, "due", "", NOW + 29 * 60_000).unwrap(), "", "not yet");
        assert!(host.posted.borrow().is_empty());

        assert_eq!(run_at(&host, "due", "", NOW + 30 * 60_000).unwrap(), "Posted 1 reminder.");
        assert_eq!(*host.posted.borrow(), ["Reminder: take the bread out"]);
        assert_eq!(run_at(&host, "list", "", NOW).unwrap(), "No reminders waiting.");
        assert_eq!(run_at(&host, "due", "", NOW + 31 * 60_000).unwrap(), "", "never posted twice");
        assert_eq!(host.posted.borrow().len(), 1);
    }

    #[test]
    fn several_due_reminders_go_out_as_one_message_in_due_order() {
        let host = FakeHost::default();
        run_at(&host, "remind", "2h second", NOW).unwrap();
        run_at(&host, "remind", "1h first", NOW).unwrap();
        run_at(&host, "due", "", NOW + 3 * 3_600_000).unwrap();
        assert_eq!(*host.posted.borrow(), ["Reminder: first\nReminder: second"]);
    }

    #[test]
    fn cancel_uses_the_listed_number_and_refuses_one_that_is_not_there() {
        let host = FakeHost::default();
        run_at(&host, "remind", "1h a", NOW).unwrap();
        run_at(&host, "remind", "2h b", NOW).unwrap();
        assert_eq!(run_at(&host, "cancel", "#1", NOW).unwrap(), "Cancelled: a");
        assert_eq!(run_at(&host, "list", "", NOW).unwrap(), "1. in 2 hours: b");
        assert!(run_at(&host, "cancel", "5", NOW).unwrap_err().contains("no reminder 5"));
        assert!(run_at(&host, "cancel", "x", NOW).unwrap_err().contains("/unremind 2"));
        assert!(run_at(&host, "cancel", "0", NOW).unwrap_err().contains("no reminder 0"));
    }

    #[test]
    fn a_space_holds_a_bounded_number_of_reminders() {
        let host = FakeHost::default();
        fill_space(&host, MAX_PENDING, "1h x", NOW);
        let refused = run_at(&host, "remind", "1h one more", NOW).unwrap_err();
        assert!(refused.contains("30 reminders"), "{refused}");
        assert!(host.total_bytes() < 64 * 1024);
    }

    #[test]
    fn a_member_holds_a_bounded_number_and_cannot_crowd_out_others() {
        let host = FakeHost::default();
        for n in 0..MAX_PER_CALLER {
            run_at(&host, "remind", "1h x", NOW + n as i64).unwrap();
        }
        let refused = run_at(&host, "remind", "1h one more", NOW).unwrap_err();
        assert!(refused.contains("You already have 5"), "{refused}");
        assert!(run_as(&host, ("bob", ""), "remind", "1h mine", NOW).is_ok(), "another member is not blocked");
    }

    #[test]
    fn one_member_cannot_list_or_cancel_anothers_reminders() {
        let host = FakeHost::default();
        run_as(&host, ("alice", "c1"), "remind", "1h alice private", NOW).unwrap();
        run_as(&host, ("bob", "c2"), "remind", "2h bob own", NOW).unwrap();
        assert_eq!(run_as(&host, ("bob", "c2"), "list", "", NOW).unwrap(), "1. in 2 hours: bob own");
        assert_eq!(run_as(&host, ("carol", "c3"), "list", "", NOW).unwrap(), "No reminders waiting.");
        assert!(run_as(&host, ("carol", "c3"), "cancel", "1", NOW).unwrap_err().contains("no reminder 1"));
        assert!(run_as(&host, ("bob", "c2"), "cancel", "2", NOW).unwrap_err().contains("no reminder 2"));
        assert_eq!(run_as(&host, ("alice", "c1"), "list", "", NOW).unwrap(), "1. in 1 hour: alice private");
    }

    #[test]
    fn a_reminder_is_armed_for_the_channel_it_was_set_in_and_fires_there() {
        let host = FakeHost::default();
        let armed = run_reply(&host, ("alice", "chan-1"), "remind", "30m take the bread out", NOW).unwrap().run_after.unwrap();
        assert_eq!(armed["command"], "due");
        assert_eq!(armed["after_ms"], 30 * 60_000);
        let id = armed["input"].as_str().unwrap().to_string();
        assert_eq!(armed["key"], id.as_str());

        let early = run_reply(&host, ("module", "chan-1"), "due", &id, NOW + 10 * 60_000).unwrap();
        assert!(host.posted.borrow().is_empty());
        assert_eq!(early.run_after.unwrap()["after_ms"], 20 * 60_000, "not yet: armed again for what is left");

        let fired = run_reply(&host, ("module", "chan-1"), "due", &id, NOW + 30 * 60_000).unwrap();
        assert_eq!(fired.output, "Posted 1 reminder.");
        assert_eq!(*host.posted.borrow(), ["Reminder: take the bread out"]);
        assert_eq!(run_as(&host, ("alice", "chan-1"), "list", "", NOW).unwrap(), "No reminders waiting.");
    }

    #[test]
    fn the_schedule_never_posts_a_reminder_tied_to_a_channel() {
        let host = FakeHost::default();
        run_as(&host, ("alice", "private-room"), "remind", "5m secret", NOW).unwrap();
        run_as(&host, ("bob", ""), "remind", "5m open", NOW).unwrap();
        assert_eq!(run_at(&host, "due", "", NOW + 10 * 60_000).unwrap(), "Posted 1 reminder.");
        assert_eq!(*host.posted.borrow(), ["Reminder: open"]);
    }

    #[test]
    fn a_tied_reminder_whose_timer_never_ran_is_dropped_not_posted_elsewhere() {
        let host = FakeHost::default();
        run_as(&host, ("alice", "private-room"), "remind", "5m secret", NOW).unwrap();
        assert_eq!(run_at(&host, "due", "", NOW + 3 * 3_600_000).unwrap(), "");
        assert!(host.posted.borrow().is_empty());
        assert_eq!(run_as(&host, ("alice", ""), "list", "", NOW).unwrap(), "No reminders waiting.");
    }

    #[test]
    fn cancelling_a_tied_reminder_cancels_its_timer() {
        let host = FakeHost::default();
        let armed = run_reply(&host, ("alice", "c1"), "remind", "1h x", NOW).unwrap().run_after.unwrap();
        let cancelled = run_reply(&host, ("alice", "c1"), "cancel", "1", NOW).unwrap().run_after.unwrap();
        assert_eq!(cancelled["cancel"], true);
        assert_eq!(cancelled["key"], armed["key"]);
    }

    #[test]
    fn a_refused_channel_post_is_tried_again_then_given_up() {
        let host = FakeHost::default();
        let id = run_reply(&host, ("alice", "c1"), "remind", "5m x", NOW).unwrap().run_after.unwrap()["input"].as_str().unwrap().to_string();
        *host.post_mode.borrow_mut() = Some(PostError::Refused("message.post rate limit reached".into()));
        let retry = run_reply(&host, ("module", "c1"), "due", &id, NOW + 5 * 60_000).unwrap();
        assert!(retry.output.contains("Trying again"), "{}", retry.output);
        assert!(retry.run_after.is_some());
        *host.post_mode.borrow_mut() = Some(PostError::Unavailable);
        assert!(run_reply(&host, ("module", "c1"), "due", &id, NOW + 3 * 3_600_000).is_err());
        assert_eq!(run_as(&host, ("alice", "c1"), "list", "", NOW).unwrap(), "No reminders waiting.");
    }

    #[test]
    fn a_reminder_kept_before_owners_were_stored_still_posts_and_belongs_to_nobody() {
        let host = FakeHost::default();
        host.map.borrow_mut().insert(format!("r:{:013}-old001", NOW + 60_000), "bake".into());
        assert_eq!(run_at(&host, "list", "", NOW).unwrap(), "No reminders waiting.");
        assert_eq!(run_at(&host, "due", "", NOW + 120_000).unwrap(), "Posted 1 reminder.");
        assert_eq!(*host.posted.borrow(), ["Reminder: bake"]);
    }

    #[test]
    fn a_member_slim_cannot_name_gets_no_reminders() {
        let host = FakeHost::default();
        assert!(run_as(&host, ("", ""), "remind", "5m a", NOW).unwrap_err().contains("did not say who you are"));
        assert!(run_as(&host, ("", ""), "list", "", NOW).is_err());
    }

    /// What overlapping /remind runs used to leave behind: a reminder stored under its own key that no index names.
    fn store_unindexed(host: &FakeHost, due: i64, salt: &str, text: &str) {
        host.map.borrow_mut().insert(format!("r:{due:013}-{salt}"), text.to_string());
    }

    #[test]
    fn a_reminder_no_index_names_is_posted_and_counts_toward_the_cap() {
        let host = FakeHost::default();
        run_at(&host, "remind", "1h kept", NOW).unwrap();
        store_unindexed(&host, NOW + 60_000, "racer", "stored by an overlapping run");
        assert_eq!(run_at(&host, "due", "", NOW + 3 * 60_000).unwrap(), "Posted 1 reminder.");
        assert_eq!(*host.posted.borrow(), ["Reminder: stored by an overlapping run"]);
        for n in 0..MAX_PENDING {
            store_unindexed(&host, NOW + n as i64, &format!("s{n:02}"), "x");
        }
        assert!(run_as(&host, ("bob", ""), "remind", "1h one more", NOW).unwrap_err().contains("30 reminders"));
    }

    #[test]
    fn a_full_space_all_due_goes_out_in_two_ticks_inside_the_call_budget() {
        let host = FakeHost::default();
        fill_space(&host, 20, "1m x", NOW);
        let later = NOW + 60 * 60_000;
        assert_eq!(run_at(&host, "due", "", later).unwrap(), "Posted 10 reminders.");
        assert_eq!(run_at(&host, "due", "", later).unwrap(), "Posted 10 reminders.");
        assert_eq!(run_as(&host, ("member0", ""), "list", "", later).unwrap(), "No reminders waiting.");
        assert!(host.map.borrow().is_empty());
    }

    #[test]
    fn a_post_the_host_refuses_keeps_the_reminder_for_the_next_tick() {
        let host = FakeHost::default();
        run_at(&host, "remind", "5m a", NOW).unwrap();
        *host.post_mode.borrow_mut() = Some(PostError::Refused("message.post rate limit reached".into()));
        let err = run_at(&host, "due", "", NOW + 10 * 60_000).unwrap_err();
        assert!(err.contains("rate limit"), "{err}");
        assert_eq!(run_at(&host, "due", "", NOW + 15 * 60_000).unwrap(), "Posted 1 reminder.");
    }

    #[test]
    fn without_a_posting_approval_the_timer_says_what_to_approve() {
        let host = FakeHost::default();
        run_at(&host, "remind", "5m a", NOW).unwrap();
        *host.post_mode.borrow_mut() = Some(PostError::Unavailable);
        assert!(run_at(&host, "due", "", NOW + 6 * 60_000).unwrap_err().contains("allow it to post"));
    }

    #[test]
    fn a_host_that_gives_no_clock_is_told_so_instead_of_setting_nonsense() {
        let host = FakeHost::default();
        assert!(run_at(&host, "remind", "5m a", 0).unwrap_err().contains("does not give modules the time"));
        assert!(run_at(&host, "due", "", 0).is_err());
    }

    #[test]
    fn storage_that_is_not_approved_is_explained() {
        let host = FakeHost::default();
        host.kv_off.set(true);
        assert!(run_at(&host, "remind", "5m a", NOW).unwrap_err().contains("allow Reminder to store data"));
    }
}
