//! Tag names: what is allowed, how input is folded to it, and which names to
//! suggest when someone mistypes one.

pub const MAX_NAME: usize = 32;
const MAX_SUGGESTIONS: usize = 3;

const RULE: &str = "Tag names use lowercase letters, digits and hyphens, up to 32 characters, and start and end with a letter or digit.";

/// Folds case and checks the rest, so `Rules` and `rules` are one tag.
pub fn normalize(raw: &str) -> Result<String, String> {
    let name = raw.trim().to_lowercase();
    if name.is_empty() {
        return Err("Give the tag a name.".to_string());
    }
    let allowed = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
    let edge = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    let fits = name.chars().count() <= MAX_NAME
        && name.chars().all(allowed)
        && name.chars().next().is_some_and(edge)
        && name.chars().last().is_some_and(edge);
    if fits {
        Ok(name)
    } else {
        Err(RULE.to_string())
    }
}

/// Up to three existing names that look like what was typed, closest first.
pub fn suggest(wanted: &str, names: &[String]) -> Vec<String> {
    let mut scored: Vec<(usize, &String)> = names
        .iter()
        .filter_map(|name| {
            let distance = distance(wanted, name);
            let related = name.contains(wanted) || wanted.contains(name.as_str());
            let near = distance <= 2 || (related && wanted.len().min(name.len()) >= 3);
            near.then_some((distance, name))
        })
        .collect();
    scored.sort();
    scored.into_iter().take(MAX_SUGGESTIONS).map(|(_, name)| name.clone()).collect()
}

/// Edit distance between two short ASCII names.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1).min(row[j] + 1).min(diagonal + usize::from(ca != *cb));
            diagonal = above;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn names_are_trimmed_and_lowercased() {
        assert_eq!(normalize("  Rules "), Ok("rules".to_string()));
        assert_eq!(normalize("faq-2"), Ok("faq-2".to_string()));
        assert_eq!(normalize("7"), Ok("7".to_string()));
    }

    #[test]
    fn bad_names_are_refused_with_the_rule() {
        assert_eq!(normalize(" "), Err("Give the tag a name.".to_string()));
        for bad in ["has space", "under_score", "-lead", "trail-", "caf\u{e9}", "semi;colon", "a/b"] {
            assert_eq!(normalize(bad), Err(RULE.to_string()), "{bad}");
        }
    }

    #[test]
    fn the_length_limit_is_thirty_two() {
        assert!(normalize(&"a".repeat(32)).is_ok());
        assert!(normalize(&"a".repeat(33)).is_err());
    }

    #[test]
    fn a_typo_suggests_the_close_names_nearest_first() {
        let known = names(&["rules", "rule", "faq", "links"]);
        assert_eq!(suggest("rulez", &known), ["rule", "rules"]);
        assert_eq!(suggest("fqa", &known), ["faq"]);
    }

    #[test]
    fn nothing_is_suggested_when_nothing_is_close() {
        assert!(suggest("zzzzzz", &names(&["rules", "faq"])).is_empty());
        assert!(suggest("x", &[]).is_empty());
    }

    #[test]
    fn at_most_three_names_are_suggested() {
        let known = names(&["aa1", "aa2", "aa3", "aa4"]);
        assert_eq!(suggest("aa", &known).len(), 3);
    }

    #[test]
    fn edit_distance_counts_insert_delete_and_replace() {
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("", "abc"), 3);
        assert_eq!(distance("same", "same"), 0);
    }
}
