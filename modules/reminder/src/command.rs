//! The four commands: set, list and cancel a reminder, and the timer's `due`.

use crate::host::{Host, KvError, PostError};
use crate::parse::{self, words};
use crate::store::{self, MAX_PENDING};

/// Reminders one tick posts in one message; the rest wait for the next tick.
const MAX_PER_POST: usize = 10;

pub fn run<H: Host>(host: &H, command: &str, input: &str, clock: (i64, &str)) -> Result<String, String> {
    let (now_ms, entropy) = clock;
    match command {
        "remind" => remind(host, input, now_ms, entropy),
        "list" => list(host, now_ms),
        "cancel" => cancel(host, input),
        "due" => due(host, now_ms),
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

fn remind<H: Host>(host: &H, input: &str, now_ms: i64, entropy: &str) -> Result<String, String> {
    need_clock(now_ms)?;
    let parsed = parse::parse(input)?;
    let due = now_ms + parsed.delay_ms;
    match store::add(host, due, entropy, &parsed.text).map_err(kv)? {
        Some(_) => Ok(format!("Reminder set for {} from now: {}", words(parsed.delay_ms), parsed.text)),
        None => Err(format!("There are already {MAX_PENDING} reminders waiting. Cancel one with /unremind first.")),
    }
}

fn list<H: Host>(host: &H, now_ms: i64) -> Result<String, String> {
    let pending = store::all(host).map_err(kv)?;
    if pending.is_empty() {
        return Ok("No reminders waiting.".to_string());
    }
    let lines: Vec<String> = pending
        .iter()
        .enumerate()
        .map(|(n, r)| format!("{}. in {}: {}", n + 1, words(r.due_ms - now_ms), r.text))
        .collect();
    Ok(lines.join("\n"))
}

fn cancel<H: Host>(host: &H, input: &str) -> Result<String, String> {
    let number: usize = input
        .trim()
        .trim_start_matches('#')
        .parse()
        .map_err(|_| "Give the number from /reminders: /unremind 2".to_string())?;
    let pending = store::all(host).map_err(kv)?;
    let chosen = number
        .checked_sub(1)
        .and_then(|n| pending.get(n))
        .ok_or_else(|| format!("There is no reminder {number}. /reminders lists them."))?;
    store::remove(host, &chosen.id).map_err(kv)?;
    Ok(format!("Cancelled: {}", chosen.text))
}

/// The timer: posts what has come due, then forgets it. A reminder is deleted
/// only after its post went out, so a refused post is tried again next tick.
fn due<H: Host>(host: &H, now_ms: i64) -> Result<String, String> {
    need_clock(now_ms)?;
    let mut ready = Vec::new();
    for reminder in store::all(host).map_err(kv)? {
        if reminder.due_ms > now_ms {
            break;
        }
        ready.push(reminder);
        if ready.len() == MAX_PER_POST {
            break;
        }
    }
    if ready.is_empty() {
        return Ok(String::new());
    }
    let lines: Vec<String> = ready.iter().map(|r| format!("Reminder: {}", r.text)).collect();
    host.post(&lines.join("\n")).map_err(post_failure)?;
    for reminder in &ready {
        store::remove(host, &reminder.id).map_err(kv)?;
    }
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

    fn run_at(host: &FakeHost, command: &str, input: &str, now: i64) -> Result<String, String> {
        host.next_run();
        run(host, command, input, (now, "ab12cd34"))
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
        for n in 0..MAX_PENDING {
            let host_ref = &host;
            host_ref.next_run();
            run(host_ref, "remind", "1h x", (NOW + n as i64, &format!("e{n:03}"))).unwrap();
        }
        let refused = run_at(&host, "remind", "1h one more", NOW).unwrap_err();
        assert!(refused.contains("20 reminders"), "{refused}");
        assert!(host.total_bytes() < 64 * 1024);
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
