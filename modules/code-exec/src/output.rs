//! What a run answers with: what the script printed, then its value or its error.
//! Each part is held to [`MAX_BYTES`], so a chatty script gets a short answer with a notice
//! where it would otherwise get the host's refusal of a response that is too large.

const MAX_BYTES: usize = 64 * 1024;
const TRUNCATED: &str = "... output truncated";

/// The console's lines so far; once the cap is hit, later lines are dropped.
#[derive(Default)]
pub struct Printed {
    text: String,
    clipped: bool,
}

impl Printed {
    pub const fn new() -> Self {
        Self {
            text: String::new(),
            clipped: false,
        }
    }

    pub fn push_line(&mut self, line: &str) {
        if self.clipped {
            return;
        }
        let separator = usize::from(!self.text.is_empty());
        let kept = clip(line, MAX_BYTES.saturating_sub(self.text.len() + separator));
        self.clipped = kept.len() < line.len();
        if kept.is_empty() && self.clipped {
            return;
        }
        if separator == 1 {
            self.text.push('\n');
        }
        self.text.push_str(kept);
    }

    /// The printed text, then `last`. Each is clipped on its own, so an error is never lost behind chatty output.
    pub fn finish(self, last: Option<&str>) -> String {
        let mut out = String::new();
        append(&mut out, &self.text, self.clipped);
        if let Some(last) = last {
            let kept = clip(last, MAX_BYTES);
            append(&mut out, kept, kept.len() < last.len());
        }
        out
    }
}

fn append(out: &mut String, text: &str, clipped: bool) {
    for part in [text, if clipped { TRUNCATED } else { "" }] {
        if part.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(part);
    }
}

fn clip(text: &str, max: usize) -> &str {
    &text[..text.floor_char_boundary(max)]
}
