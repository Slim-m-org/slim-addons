//! Pretty-prints a JSON string with 2-space indentation, changing nothing but whitespace.
//!
//! The input is validated with serde_json and then re-indented token by token,
//! never rebuilt as a `Value`: that would collapse duplicate keys and rewrite
//! number text (`1.50`, `1E2`, big integers).

use serde::de::IgnoredAny;

const INDENT: &str = "  ";

/// Validates `input` as JSON and re-indents it with 2 spaces. On a parse
/// failure, returns the underlying `serde_json` error text, which already
/// carries a `line X column Y` location.
pub fn pretty_print(input: &str) -> Result<String, String> {
    serde_json::from_str::<IgnoredAny>(input).map_err(|err| err.to_string())?;
    Ok(reindent(input))
}

fn reindent(valid_json: &str) -> String {
    let mut out = String::with_capacity(valid_json.len() * 2);
    let mut depth = 0usize;
    let mut chars = valid_json.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => copy_string(&mut out, &mut chars),
            '{' | '[' => {
                out.push(c);
                skip_space(&mut chars);
                if matches!(chars.peek(), Some('}' | ']')) {
                    out.extend(chars.next());
                } else {
                    depth += 1;
                    newline(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(c);
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    out
}

fn copy_string(out: &mut String, chars: &mut std::iter::Peekable<std::str::Chars>) {
    out.push('"');
    while let Some(c) = chars.next() {
        out.push(c);
        match c {
            '\\' => out.extend(chars.next()),
            '"' => return,
            _ => {}
        }
    }
}

fn skip_space(chars: &mut std::iter::Peekable<std::str::Chars>) {
    while chars.next_if(|c| c.is_whitespace()).is_some() {}
}

fn newline(out: &mut String, depth: usize) {
    out.push('\n');
    out.push_str(&INDENT.repeat(depth));
}

#[cfg(test)]
mod tests {
    use super::pretty_print;

    #[test]
    fn indents_nested_values_with_two_spaces() {
        let out = pretty_print(r#"{"a":[1,2,{"b":null}],"c":"x"}"#).unwrap();
        assert_eq!(out, "{\n  \"a\": [\n    1,\n    2,\n    {\n      \"b\": null\n    }\n  ],\n  \"c\": \"x\"\n}");
    }

    #[test]
    fn empty_containers_stay_on_one_line() {
        assert_eq!(pretty_print(r#"{"a":{},"b":[ ]}"#).unwrap(), "{\n  \"a\": {},\n  \"b\": []\n}");
    }

    #[test]
    fn number_text_survives_untouched() {
        for number in ["100000000000000000000", "3.14159265358979323846", "1.50", "-0", "1E2"] {
            let out = pretty_print(&format!(r#"{{"n":{number}}}"#)).unwrap();
            assert_eq!(out, format!("{{\n  \"n\": {number}\n}}"));
        }
    }

    #[test]
    fn duplicate_keys_are_kept_in_order() {
        assert_eq!(pretty_print(r#"{"a":1,"a":2}"#).unwrap(), "{\n  \"a\": 1,\n  \"a\": 2\n}");
    }

    #[test]
    fn strings_keep_their_escapes_and_brackets() {
        let source = r#"{"k":"a \" , : { [ \\ é é"}"#;
        assert_eq!(pretty_print(source).unwrap(), format!("{{\n  {}\n}}", &source[1..source.len() - 1].replacen(":", ": ", 1)));
    }

    #[test]
    fn a_bare_scalar_is_returned_as_is() {
        assert_eq!(pretty_print("  42 \n").unwrap(), "42");
    }

    #[test]
    fn invalid_json_reports_where() {
        let err = pretty_print("{\n  \"a\": }").unwrap_err();
        assert!(err.contains("line 2"), "{err}");
    }
}
