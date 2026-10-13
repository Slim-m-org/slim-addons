//! The checklist, its limits, and the actions that change it.
//!
//! The whole list rides in the scene's opaque `state`, so it is shared by
//! everyone looking at the message and needs no host storage.

use serde::{Deserialize, Serialize};

pub const MAX_TITLE: usize = 40;
pub const MAX_ITEMS: usize = 24;
pub const MAX_ITEM: usize = 40;
pub const MAX_NOTES: usize = 280;
pub const DEFAULT_TITLE: &str = "Checklist";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Item {
    #[serde(rename = "s")]
    pub text: String,
    #[serde(rename = "d")]
    pub done: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct List {
    #[serde(rename = "t")]
    pub title: String,
    #[serde(rename = "i")]
    pub items: Vec<Item>,
    #[serde(rename = "n")]
    pub notes: String,
}

/// A frame to draw: the list, plus the one line saying why the last action did nothing, if it did nothing.
pub struct Frame {
    pub list: List,
    pub status: Option<String>,
}

#[derive(Deserialize)]
struct Action {
    action: String,
    #[serde(default)]
    state: String,
}

impl List {
    fn new(title: &str) -> Self {
        List { title: title.to_string(), items: Vec::new(), notes: String::new() }
    }

    /// Reads the state this module wrote last frame; anything unreadable is an empty list, and anything over a limit is cut back to it.
    fn from_state(state: &str) -> Self {
        let mut list = serde_json::from_str::<List>(state).unwrap_or_else(|_| List::new(DEFAULT_TITLE));
        list.title = clip(&list.title, MAX_TITLE);
        list.items.truncate(MAX_ITEMS);
        for item in &mut list.items {
            item.text = clip(&item.text, MAX_ITEM);
        }
        list.notes = clip(&list.notes, MAX_NOTES);
        if list.title.is_empty() {
            list.title = DEFAULT_TITLE.to_string();
        }
        list
    }

    pub fn state(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn done_count(&self) -> usize {
        self.items.iter().filter(|i| i.done).count()
    }
}

pub fn apply(command: &str, input: &str) -> Result<String, String> {
    match command {
        "checklist" => Ok(crate::scene::render(&checklist(input))),
        other => Err(format!("unknown command: {other}")),
    }
}

fn checklist(input: &str) -> Frame {
    let trimmed = input.trim();
    match serde_json::from_str::<Action>(trimmed) {
        Ok(a) if trimmed.starts_with('{') => act(List::from_state(&a.state), &a.action),
        // A slash command's text, or a launch with none, names a new list.
        _ => launch(trimmed),
    }
}

fn launch(text: &str) -> Frame {
    let title = tidy(text);
    if title.is_empty() {
        return Frame { list: List::new(DEFAULT_TITLE), status: None };
    }
    match check_len(&title, MAX_TITLE, "title") {
        Ok(()) => Frame { list: List::new(&title), status: None },
        Err(why) => Frame { list: List::new(DEFAULT_TITLE), status: Some(why) },
    }
}

fn act(mut list: List, action: &str) -> Frame {
    let result = match action.split_once(':') {
        Some(("t", n)) => index(&list, n).map(|i| list.items[i].done = !list.items[i].done),
        Some(("x", n)) => index(&list, n).map(|i| drop(list.items.remove(i))),
        Some(("add", text)) => add(&mut list, text),
        Some(("title", text)) => set_title(&mut list, text),
        Some(("notes", text)) => set_notes(&mut list, text),
        _ if action == "clear done" => Ok(list.items.retain(|i| !i.done)),
        _ => Ok(()),
    };
    Frame { list, status: result.err() }
}

/// A tap names an item by position; a stale tap on a slot that has since gone is ignored, not an error to shout about.
fn index(list: &List, raw: &str) -> Result<usize, String> {
    match raw.trim().parse::<usize>() {
        Ok(i) if i < list.items.len() => Ok(i),
        _ => Err("that item is already gone".to_string()),
    }
}

fn add(list: &mut List, raw: &str) -> Result<(), String> {
    let text = tidy(raw);
    if text.is_empty() {
        return Err("type something to add".to_string());
    }
    check_len(&text, MAX_ITEM, "item")?;
    if list.items.len() >= MAX_ITEMS {
        return Err(format!("the list is full ({MAX_ITEMS} items) - remove or clear some first"));
    }
    list.items.push(Item { text, done: false });
    Ok(())
}

fn set_title(list: &mut List, raw: &str) -> Result<(), String> {
    let title = tidy(raw);
    if title.is_empty() {
        return Err("a title cannot be empty".to_string());
    }
    check_len(&title, MAX_TITLE, "title")?;
    list.title = title;
    Ok(())
}

fn set_notes(list: &mut List, raw: &str) -> Result<(), String> {
    let notes = tidy(raw);
    check_len(&notes, MAX_NOTES, "notes")?;
    list.notes = notes;
    Ok(())
}

fn check_len(text: &str, max: usize, what: &str) -> Result<(), String> {
    let len = text.chars().count();
    if len > max {
        return Err(format!("{what} is {len} characters, the most is {max} - nothing was changed"));
    }
    Ok(())
}

/// Trimmed, with control characters (a pasted newline, a tab) turned into spaces so one entry stays one line.
fn tidy(raw: &str) -> String {
    let spaced: String = raw.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    spaced.trim().to_string()
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(list: &List, action: &str) -> Frame {
        let input = serde_json::json!({"action": action, "state": list.state()}).to_string();
        checklist(&input)
    }

    fn with(items: &[(&str, bool)]) -> List {
        let mut list = List::new("Trip");
        list.items = items.iter().map(|(s, d)| Item { text: s.to_string(), done: *d }).collect();
        list
    }

    #[test]
    fn launch_with_no_input_is_an_empty_default_list() {
        let f = checklist("");
        assert_eq!(f.list, List::new(DEFAULT_TITLE));
        assert!(f.status.is_none());
    }

    #[test]
    fn launch_with_text_uses_it_as_the_title() {
        assert_eq!(checklist("  Packing  ").list.title, "Packing");
    }

    #[test]
    fn launch_with_an_overlong_title_explains_and_falls_back() {
        let f = checklist(&"a".repeat(MAX_TITLE + 1));
        assert_eq!(f.list.title, DEFAULT_TITLE);
        assert!(f.status.unwrap().contains("title"));
    }

    #[test]
    fn launch_with_text_that_looks_like_json_but_is_not_an_action_is_a_title() {
        assert_eq!(checklist("{not json").list.title, "{not json");
    }

    #[test]
    fn add_appends_an_unticked_item() {
        let f = run(&with(&[]), "add:milk");
        assert_eq!(f.list.items, vec![Item { text: "milk".into(), done: false }]);
        assert!(f.status.is_none());
    }

    #[test]
    fn add_keeps_colons_in_the_text() {
        assert_eq!(run(&with(&[]), "add:call: bob").list.items[0].text, "call: bob");
    }

    #[test]
    fn add_turns_newlines_into_spaces() {
        assert_eq!(run(&with(&[]), "add:a\nb").list.items[0].text, "a b");
    }

    #[test]
    fn add_of_nothing_is_explained() {
        let f = run(&with(&[]), "add:   ");
        assert!(f.list.items.is_empty());
        assert!(f.status.unwrap().contains("something"));
    }

    #[test]
    fn add_at_the_item_length_limit_works_and_one_over_is_refused_whole() {
        let ok = run(&with(&[]), &format!("add:{}", "x".repeat(MAX_ITEM)));
        assert_eq!(ok.list.items[0].text.chars().count(), MAX_ITEM);
        let over = run(&with(&[]), &format!("add:{}", "x".repeat(MAX_ITEM + 1)));
        assert!(over.list.items.is_empty());
        assert!(over.status.unwrap().contains(&MAX_ITEM.to_string()));
    }

    #[test]
    fn the_length_limit_counts_characters_not_bytes() {
        let f = run(&with(&[]), &format!("add:{}", "e\u{301}".repeat(MAX_ITEM / 2)));
        assert_eq!(f.list.items.len(), 1);
    }

    #[test]
    fn add_past_the_item_count_limit_is_refused_with_a_reason() {
        let full: Vec<(&str, bool)> = (0..MAX_ITEMS).map(|_| ("x", false)).collect();
        let f = run(&with(&full), "add:one more");
        assert_eq!(f.list.items.len(), MAX_ITEMS);
        assert!(f.status.unwrap().contains("full"));
    }

    #[test]
    fn tap_ticks_then_unticks() {
        let list = with(&[("a", false), ("b", false)]);
        let ticked = run(&list, "t:1");
        assert_eq!(ticked.list.items.iter().map(|i| i.done).collect::<Vec<_>>(), [false, true]);
        let unticked = run(&ticked.list, "t:1");
        assert!(!unticked.list.items[1].done);
    }

    #[test]
    fn tap_on_a_missing_item_changes_nothing_and_says_so() {
        let list = with(&[("a", false)]);
        for action in ["t:5", "t:x", "t:", "x:9", "x:-1"] {
            let f = run(&list, action);
            assert_eq!(f.list, list, "{action}");
            assert!(f.status.is_some(), "{action}");
        }
    }

    #[test]
    fn remove_drops_just_that_item() {
        let f = run(&with(&[("a", false), ("b", true), ("c", false)]), "x:1");
        assert_eq!(f.list.items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["a", "c"]);
    }

    #[test]
    fn clear_done_keeps_only_unticked_items() {
        let f = run(&with(&[("a", true), ("b", false), ("c", true)]), "clear done");
        assert_eq!(f.list.items, vec![Item { text: "b".into(), done: false }]);
        assert!(f.status.is_none());
    }

    #[test]
    fn title_can_be_renamed() {
        assert_eq!(run(&with(&[]), "title:Groceries").list.title, "Groceries");
    }

    #[test]
    fn title_empty_or_overlong_is_refused_and_keeps_the_old_one() {
        let empty = run(&with(&[]), "title: ");
        assert_eq!(empty.list.title, "Trip");
        assert!(empty.status.is_some());
        let over = run(&with(&[]), &format!("title:{}", "t".repeat(MAX_TITLE + 1)));
        assert_eq!(over.list.title, "Trip");
        assert!(over.status.unwrap().contains("title"));
    }

    #[test]
    fn notes_are_set_and_can_be_emptied() {
        let set = run(&with(&[]), "notes:bring cash");
        assert_eq!(set.list.notes, "bring cash");
        let cleared = run(&set.list, "notes:");
        assert_eq!(cleared.list.notes, "");
        assert!(cleared.status.is_none());
    }

    #[test]
    fn notes_at_the_limit_work_and_over_it_are_refused_keeping_the_old_notes() {
        let mut list = with(&[]);
        list.notes = "keep".into();
        assert_eq!(run(&list, &format!("notes:{}", "n".repeat(MAX_NOTES))).list.notes.chars().count(), MAX_NOTES);
        let over = run(&list, &format!("notes:{}", "n".repeat(MAX_NOTES + 1)));
        assert_eq!(over.list.notes, "keep");
        assert!(over.status.unwrap().contains("notes"));
    }

    #[test]
    fn unknown_and_malformed_actions_keep_the_list() {
        let list = with(&[("a", true)]);
        for action in ["", "dance", "nocolon", ":", "zz:1"] {
            let f = run(&list, action);
            assert_eq!(f.list, list, "{action:?}");
            assert!(f.status.is_none(), "{action:?}");
        }
    }

    #[test]
    fn a_broken_state_becomes_an_empty_list_instead_of_an_error() {
        let f = checklist(r#"{"action":"add:a","state":"garbage"}"#);
        assert_eq!(f.list.items.len(), 1);
        assert_eq!(f.list.title, DEFAULT_TITLE);
    }

    #[test]
    fn a_state_over_the_limits_is_cut_back_when_read() {
        let mut list = with(&[]);
        list.items = (0..MAX_ITEMS + 5).map(|_| Item { text: "y".repeat(MAX_ITEM + 9), done: true }).collect();
        list.title = "z".repeat(MAX_TITLE + 9);
        list.notes = "n".repeat(MAX_NOTES + 9);
        let read = List::from_state(&list.state());
        assert_eq!(read.items.len(), MAX_ITEMS);
        assert_eq!(read.items[0].text.chars().count(), MAX_ITEM);
        assert_eq!(read.title.chars().count(), MAX_TITLE);
        assert_eq!(read.notes.chars().count(), MAX_NOTES);
    }

    #[test]
    fn the_worst_case_state_stays_far_under_the_response_ceiling() {
        let mut list = with(&[]);
        list.title = "\u{1F600}".repeat(MAX_TITLE);
        list.notes = "\"".repeat(MAX_NOTES);
        list.items = (0..MAX_ITEMS).map(|_| Item { text: "\u{1F600}".repeat(MAX_ITEM), done: true }).collect();
        assert!(list.state().len() < 8 * 1024, "{}", list.state().len());
    }

    #[test]
    fn state_round_trips() {
        let list = with(&[("a", true), ("b", false)]);
        assert_eq!(List::from_state(&list.state()), list);
    }

    #[test]
    fn unknown_command_is_an_error() {
        assert!(apply("nope", "").is_err());
    }
}
