//! The tag set over `kv.store`.
//!
//! Layout: one key per tag (`t:<name>`, value is the text) plus one index key
//! (`i`, newline-joined sorted names). Listing is one read whatever the size,
//! and a single tag is one read. The tag key is the truth; the index only
//! feeds listing and the count, so an index that fell behind a crash never
//! hides a tag from `/tag name`.

use crate::host::{Host, KvError};
use crate::name;

pub const MAX_TAGS: usize = 40;
pub const MAX_CHARS: usize = 1000;
/// With 40 tags and the index this stays under kv.store's 64 KiB total, with room to spare.
pub const MAX_BYTES: usize = 1500;
const INDEX: &str = "i";

fn key(name: &str) -> String {
    format!("t:{name}")
}

pub struct Tags<'a, H: Host> {
    host: &'a H,
}

impl<'a, H: Host> Tags<'a, H> {
    pub fn new(host: &'a H) -> Self {
        Self { host }
    }

    pub fn names(&self) -> Result<Vec<String>, String> {
        let index = self.host.get(INDEX).map_err(|e| e.text())?;
        Ok(index.unwrap_or_default().lines().filter(|l| !l.is_empty()).map(str::to_string).collect())
    }

    pub fn get(&self, name: &str) -> Result<Option<String>, String> {
        self.host.get(&key(name)).map_err(|e| e.text())
    }

    pub fn add(&self, name: &str, text: &str) -> Result<(), String> {
        let text = check_text(text)?;
        if self.get(name)?.is_some() {
            return Err(format!("A tag named {name} already exists. Use /tagedit to change it."));
        }
        let mut names = self.names()?;
        if names.len() >= MAX_TAGS {
            return Err(format!("This space has {MAX_TAGS} tags, which is the limit. Remove one first."));
        }
        self.host.set(&key(name), &text).map_err(|e| e.text())?;
        names.push(name.to_string());
        self.write_index(names)
    }

    pub fn edit(&self, name: &str, text: &str) -> Result<(), String> {
        let text = check_text(text)?;
        if self.get(name)?.is_none() {
            return Err(self.unknown(name));
        }
        self.host.set(&key(name), &text).map_err(|e| e.text())
    }

    pub fn remove(&self, name: &str) -> Result<(), String> {
        let stored = self.get(name)?.is_some();
        let mut names = self.names()?;
        let indexed = names.iter().any(|n| n == name);
        if !stored && !indexed {
            return Err(self.unknown_in(name, &names));
        }
        self.host.delete(&key(name)).map_err(|e| e.text())?;
        names.retain(|n| n != name);
        self.write_index(names)
    }

    /// The error for a name nobody has, with close names when there are any.
    pub fn unknown(&self, name: &str) -> String {
        match self.names() {
            Ok(names) => self.unknown_in(name, &names),
            Err(e) => e,
        }
    }

    fn unknown_in(&self, name: &str, names: &[String]) -> String {
        let close = name::suggest(name, names);
        if close.is_empty() {
            format!("There is no tag named {name}. /tag lists them.")
        } else {
            format!("There is no tag named {name}. Did you mean {}?", close.join(", "))
        }
    }

    fn write_index(&self, mut names: Vec<String>) -> Result<(), String> {
        names.sort();
        names.dedup();
        self.host.set(INDEX, &names.join("\n")).map_err(|e: KvError| e.text())
    }
}

/// Trimmed text that fits both the character limit people see and the byte limit storage needs.
pub fn check_text(raw: &str) -> Result<String, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Err("The tag needs some text.".to_string());
    }
    let chars = text.chars().count();
    if chars > MAX_CHARS {
        return Err(format!("That text is {chars} characters. A tag holds up to {MAX_CHARS}."));
    }
    if text.len() > MAX_BYTES {
        return Err(format!("That text is too big to store. A tag holds up to {MAX_BYTES} bytes, which is fewer characters outside plain latin text."));
    }
    Ok(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeHost;

    #[test]
    fn add_then_get_round_trips_and_lists_sorted() {
        let host = FakeHost::default();
        let tags = Tags::new(&host);
        tags.add("rules", "Be kind.").unwrap();
        tags.add("faq", "Ask in #help.").unwrap();
        assert_eq!(tags.get("rules").unwrap(), Some("Be kind.".to_string()));
        assert_eq!(tags.names().unwrap(), ["faq", "rules"]);
        assert_eq!(host.map.borrow().keys().cloned().collect::<Vec<_>>(), ["i", "t:faq", "t:rules"]);
    }

    #[test]
    fn a_taken_name_is_refused_and_the_text_is_kept() {
        let host = FakeHost::default();
        let tags = Tags::new(&host);
        tags.add("rules", "Be kind.").unwrap();
        let err = tags.add("rules", "other").unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert_eq!(tags.get("rules").unwrap(), Some("Be kind.".to_string()));
    }

    #[test]
    fn edit_replaces_text_and_an_unknown_name_suggests_close_ones() {
        let host = FakeHost::default();
        let tags = Tags::new(&host);
        tags.add("rules", "v1").unwrap();
        tags.edit("rules", "v2").unwrap();
        assert_eq!(tags.get("rules").unwrap(), Some("v2".to_string()));
        let err = tags.edit("rulez", "x").unwrap_err();
        assert_eq!(err, "There is no tag named rulez. Did you mean rules?");
        assert!(tags.edit("zzzzzz", "x").unwrap_err().ends_with("/tag lists them."));
    }

    #[test]
    fn remove_drops_the_tag_and_its_index_entry() {
        let host = FakeHost::default();
        let tags = Tags::new(&host);
        tags.add("a", "1").unwrap();
        tags.add("b", "2").unwrap();
        tags.remove("a").unwrap();
        assert_eq!(tags.names().unwrap(), ["b"]);
        assert_eq!(tags.get("a").unwrap(), None);
        assert!(tags.remove("a").unwrap_err().contains("no tag named a"));
    }

    #[test]
    fn a_dangling_index_entry_can_still_be_removed() {
        let host = FakeHost::default();
        host.map.borrow_mut().insert("i".into(), "ghost".into());
        let tags = Tags::new(&host);
        tags.remove("ghost").unwrap();
        assert!(tags.names().unwrap().is_empty());
    }

    #[test]
    fn a_tag_missing_from_the_index_is_still_found() {
        let host = FakeHost::default();
        host.map.borrow_mut().insert("t:lost".into(), "text".into());
        assert_eq!(Tags::new(&host).get("lost").unwrap(), Some("text".to_string()));
    }

    #[test]
    fn text_limits_count_characters_and_bytes() {
        assert!(check_text(&"a".repeat(MAX_CHARS)).is_ok());
        assert!(check_text(&"a".repeat(MAX_CHARS + 1)).unwrap_err().contains("1001 characters"));
        assert!(check_text(&"\u{3042}".repeat(600)).unwrap_err().contains("too big to store"));
        assert!(check_text("   ").is_err());
        assert_eq!(check_text("  hi  "), Ok("hi".to_string()));
    }

    #[test]
    fn the_forty_first_tag_is_refused() {
        let host = FakeHost::default();
        let tags = Tags::new(&host);
        for i in 0..MAX_TAGS {
            tags.add(&format!("t{i}"), "x").unwrap();
            host.next_run();
        }
        let err = tags.add("one-more", "x").unwrap_err();
        assert!(err.contains("limit"), "{err}");
    }

    #[test]
    fn a_full_set_of_maximum_tags_fits_kv_store_with_room_to_spare() {
        let host = FakeHost::default();
        let tags = Tags::new(&host);
        for i in 0..MAX_TAGS {
            let name = format!("{}{i:02}", "w".repeat(name::MAX_NAME - 2));
            tags.add(&name, &"\u{3042}".repeat(MAX_BYTES / 3)).unwrap();
            host.next_run();
        }
        assert!(host.total_bytes() < 64 * 1024 - 2 * 1024, "{}", host.total_bytes());
        assert!(host.map.borrow().len() <= 41);
    }

    #[test]
    fn each_operation_stays_far_inside_the_per_run_call_budget() {
        let host = FakeHost::default();
        let tags = Tags::new(&host);
        let calls = |f: &dyn Fn()| {
            host.next_run();
            f();
            host.calls.get()
        };
        assert_eq!(calls(&|| tags.add("a", "1").unwrap()), 4);
        assert_eq!(calls(&|| tags.edit("a", "2").unwrap()), 2);
        assert_eq!(calls(&|| drop(tags.names().unwrap())), 1);
        assert_eq!(calls(&|| tags.remove("a").unwrap()), 4);
    }

    #[test]
    fn storage_errors_come_back_as_plain_sentences() {
        let host = FakeHost::default();
        host.kv_off.set(true);
        let err = Tags::new(&host).add("a", "1").unwrap_err();
        assert_eq!(err, "An admin needs to allow Tags to store data in the Dock.");
    }
}
