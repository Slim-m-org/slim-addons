//! The commands: parse the input, call the tag set, and decide what to say.
//!
//! A slash command's output is posted by the composer as a plain message from the
//! person, so anything meant for the channel goes out through `message.post`
//! (which carries the "via Tags" footer) and the command answers with empty
//! output. Only where `message.post` is missing does the text itself become the output.

use crate::host::{Host, PostError};
use crate::name;
use crate::store::Tags;

/// What the command hands back to slim: the output text, or a private error.
pub type Reply = Result<String, String>;

pub fn run<H: Host>(host: &H, command: &str, input: &str) -> Reply {
    let tags = Tags::new(host);
    match command {
        "tag" => show(host, &tags, input),
        "add" => manage(host, input, "/tagadd name text", |n, t| tags.add(n, t).map(|_| format!("Added tag {n}."))),
        "edit" => manage(host, input, "/tagedit name text", |n, t| tags.edit(n, t).map(|_| format!("Updated tag {n}."))),
        "remove" => remove(host, &tags, input),
        other => Err(format!("unknown command: {other}")),
    }
}

fn show<H: Host>(host: &H, tags: &Tags<H>, input: &str) -> Reply {
    let Some(word) = input.split_whitespace().next() else {
        return list(host, tags);
    };
    let name = name::normalize(word)?;
    match tags.get(&name)? {
        Some(text) => say(host, &text),
        None => Err(tags.unknown(&name)),
    }
}

fn list<H: Host>(host: &H, tags: &Tags<H>) -> Reply {
    let names = tags.names()?;
    if names.is_empty() {
        return Err("There are no tags yet. Someone who manages tags can add one with /tagadd.".to_string());
    }
    say(host, &format!("Tags: {}. Use /tag name to post one.", names.join(", ")))
}

fn manage<H: Host>(host: &H, input: &str, usage: &str, apply: impl FnOnce(&str, &str) -> Reply) -> Reply {
    let (raw_name, text) = input.trim().split_once(char::is_whitespace).unwrap_or((input.trim(), ""));
    if raw_name.is_empty() || text.trim().is_empty() {
        return Err(format!("Usage: {usage}"));
    }
    let name = name::normalize(raw_name)?;
    confirm(host, &apply(&name, text)?)
}

fn remove<H: Host>(host: &H, tags: &Tags<H>, input: &str) -> Reply {
    let Some(word) = input.split_whitespace().next() else {
        return Err("Usage: /tagremove name".to_string());
    };
    let name = name::normalize(word)?;
    tags.remove(&name)?;
    confirm(host, &format!("Removed tag {name}."))
}

/// Posts for the channel to see; plain output when `message.post` is not there.
fn say<H: Host>(host: &H, text: &str) -> Reply {
    match host.post(text) {
        Ok(()) => Ok(String::new()),
        Err(PostError::Unavailable) => Ok(text.to_string()),
        Err(PostError::Refused(why)) => Err(format!("Could not post: {why}.")),
    }
}

/// Tells the channel a change happened. The change is already saved, so a post
/// that fails or is missing must not turn it into an error, and plain output
/// would be posted as the person's own words, so it stays quiet instead.
fn confirm<H: Host>(host: &H, text: &str) -> Reply {
    let _ = host.post(text);
    Ok(String::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeHost;

    fn ok(host: &FakeHost, command: &str, input: &str) -> String {
        run(host, command, input).unwrap()
    }

    #[test]
    fn adding_then_posting_a_tag_posts_its_text_and_leaves_no_output() {
        let host = FakeHost::default();
        assert_eq!(ok(&host, "add", "Rules Be kind to each other."), "");
        assert_eq!(ok(&host, "tag", "rules"), "");
        assert_eq!(*host.posted.borrow(), ["Added tag rules.", "Be kind to each other."]);
    }

    #[test]
    fn the_list_is_posted_sorted_and_an_empty_one_is_a_private_hint() {
        let host = FakeHost::default();
        assert!(run(&host, "tag", "").unwrap_err().contains("no tags yet"));
        ok(&host, "add", "b two");
        ok(&host, "add", "a one");
        host.posted.borrow_mut().clear();
        assert_eq!(ok(&host, "tag", ""), "");
        assert_eq!(*host.posted.borrow(), ["Tags: a, b. Use /tag name to post one."]);
    }

    #[test]
    fn without_message_post_the_text_becomes_the_output() {
        let host = FakeHost::default();
        ok(&host, "add", "faq ask in help");
        *host.post_mode.borrow_mut() = Some(PostError::Unavailable);
        assert_eq!(ok(&host, "tag", "faq"), "ask in help");
    }

    #[test]
    fn a_refused_post_is_reported_not_swallowed() {
        let host = FakeHost::default();
        ok(&host, "add", "faq ask in help");
        *host.post_mode.borrow_mut() = Some(PostError::Refused("message.post rate limit reached".into()));
        assert_eq!(run(&host, "tag", "faq").unwrap_err(), "Could not post: message.post rate limit reached.");
    }

    #[test]
    fn a_change_survives_a_missing_or_refused_confirmation() {
        let host = FakeHost::default();
        *host.post_mode.borrow_mut() = Some(PostError::Unavailable);
        assert_eq!(ok(&host, "add", "faq ask"), "");
        *host.post_mode.borrow_mut() = Some(PostError::Refused("slow mode".into()));
        assert_eq!(ok(&host, "edit", "faq ask more"), "");
        assert_eq!(Tags::new(&host).get("faq").unwrap(), Some("ask more".to_string()));
    }

    #[test]
    fn unknown_names_get_suggestions() {
        let host = FakeHost::default();
        ok(&host, "add", "rules be kind");
        assert_eq!(run(&host, "tag", "rulez").unwrap_err(), "There is no tag named rulez. Did you mean rules?");
        assert!(run(&host, "remove", "nope").unwrap_err().contains("no tag named nope"));
    }

    #[test]
    fn usage_and_validation_errors_are_short() {
        let host = FakeHost::default();
        assert_eq!(run(&host, "add", "").unwrap_err(), "Usage: /tagadd name text");
        assert_eq!(run(&host, "add", "onlyname").unwrap_err(), "Usage: /tagadd name text");
        assert_eq!(run(&host, "edit", "x").unwrap_err(), "Usage: /tagedit name text");
        assert_eq!(run(&host, "remove", "  ").unwrap_err(), "Usage: /tagremove name");
        assert!(run(&host, "add", "bad_name text").unwrap_err().starts_with("Tag names use"));
        assert!(run(&host, "add", &format!("big {}", "x".repeat(1001))).unwrap_err().contains("up to 1000"));
    }

    #[test]
    fn a_taken_name_says_so_and_points_at_edit() {
        let host = FakeHost::default();
        ok(&host, "add", "rules one");
        assert_eq!(
            run(&host, "add", "RULES two").unwrap_err(),
            "A tag named rules already exists. Use /tagedit to change it."
        );
    }

    #[test]
    fn the_text_keeps_its_inner_spacing() {
        let host = FakeHost::default();
        ok(&host, "add", "links  a  b   c ");
        assert_eq!(Tags::new(&host).get("links").unwrap(), Some("a  b   c".to_string()));
    }

    #[test]
    fn a_denied_store_is_explained_for_every_command() {
        let host = FakeHost::default();
        host.kv_off.set(true);
        for (command, input) in [("tag", ""), ("tag", "x"), ("add", "x y"), ("edit", "x y"), ("remove", "x")] {
            assert_eq!(
                run(&host, command, input).unwrap_err(),
                "An admin needs to allow Tags to store data in the Dock.",
                "{command}"
            );
        }
    }

    #[test]
    fn every_command_stays_inside_the_kv_call_budget() {
        let host = FakeHost::default();
        for command in ["tag", "add", "edit", "remove"] {
            host.next_run();
            let _ = run(&host, command, "x y");
            assert!(host.calls.get() <= 6, "{command}: {}", host.calls.get());
        }
    }

    #[test]
    fn an_unknown_command_is_an_error() {
        assert!(run(&FakeHost::default(), "nope", "").is_err());
    }
}
