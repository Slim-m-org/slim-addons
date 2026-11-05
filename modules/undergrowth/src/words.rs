//! Counted nouns, so a sentence never says "1 leaves".

/// `n` and the noun that fits it: `count(1, "leaf", "leaves")` is "1 leaf".
pub fn count<N: Into<i64>>(n: N, one: &str, many: &str) -> String {
    let n = n.into();
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_is_singular_and_everything_else_plural() {
        assert_eq!(count(1, "leaf", "leaves"), "1 leaf");
        assert_eq!(count(0, "leaf", "leaves"), "0 leaves");
        assert_eq!(count(7u32, "shower", "showers"), "7 showers");
    }
}
