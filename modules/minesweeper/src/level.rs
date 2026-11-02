//! The board sizes: three presets and a custom one, each a width, a height and a mine count.

pub const MIN_SIDE: usize = 5;
pub const MAX_COLS: usize = 30;
pub const MAX_ROWS: usize = 24;
/// Most of a board may not be mines, or the opening tap has nowhere safe to land.
const MAX_MINE_PERCENT: usize = 35;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    pub cols: usize,
    pub rows: usize,
    pub mines: usize,
}

pub const BEGINNER: Level = Level { cols: 9, rows: 9, mines: 10 };
pub const INTERMEDIATE: Level = Level { cols: 16, rows: 16, mines: 40 };
pub const EXPERT: Level = Level { cols: 30, rows: 16, mines: 99 };

impl Level {
    pub fn cells(&self) -> usize {
        self.cols * self.rows
    }

    /// The name a preset goes by in controls, state and the scoreboard; a custom board has none.
    pub fn preset_name(&self) -> Option<&'static str> {
        [(BEGINNER, "beginner"), (INTERMEDIATE, "intermediate"), (EXPERT, "expert")]
            .into_iter()
            .find(|(level, _)| level == self)
            .map(|(_, name)| name)
    }

    /// What the state carries: a preset's name, or `WxHxM`.
    pub fn code(&self) -> String {
        self.preset_name().map_or_else(|| format!("{}x{}x{}", self.cols, self.rows, self.mines), str::to_owned)
    }

    pub fn from_code(code: &str) -> Option<Level> {
        match code {
            "beginner" => Some(BEGINNER),
            "intermediate" => Some(INTERMEDIATE),
            "expert" => Some(EXPERT),
            _ => {
                let mut parts = code.split('x').map(|p| p.parse::<usize>().ok());
                let (cols, rows, mines) = (parts.next()??, parts.next()??, parts.next()??);
                parts.next().is_none().then(|| custom(cols, rows, mines)).flatten()
            }
        }
    }

    pub fn from_preset(name: &str) -> Option<Level> {
        ["beginner", "intermediate", "expert"].contains(&name).then(|| Self::from_code(name)).flatten()
    }
}

/// A board of the asked size, or None when it is outside what a scene can draw and a first tap can survive.
pub fn custom(cols: usize, rows: usize, mines: usize) -> Option<Level> {
    let cells_ok = (MIN_SIDE..=MAX_COLS).contains(&cols) && (MIN_SIDE..=MAX_ROWS).contains(&rows);
    let level = Level { cols, rows, mines };
    (cells_ok && mines >= 1 && mines * 100 <= level.cells() * MAX_MINE_PERCENT).then_some(level)
}

/// `20x12 30` or `20 12 30`: columns, rows and mines typed into the custom field.
pub fn parse_custom(text: &str) -> Result<Level, String> {
    let numbers: Vec<usize> = text
        .split(|c: char| c == 'x' || c == 'X' || c == ',' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<usize>().map_err(|_| format!("{part} is not a number")))
        .collect::<Result<_, _>>()?;
    let [cols, rows, mines] = numbers[..] else {
        return Err("type columns, rows and mines, like 20x12 30".to_owned());
    };
    custom(cols, rows, mines).ok_or_else(|| {
        format!("columns {MIN_SIDE} to {MAX_COLS}, rows {MIN_SIDE} to {MAX_ROWS}, and at most {MAX_MINE_PERCENT}% mines")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preset_round_trips_through_its_code() {
        for level in [BEGINNER, INTERMEDIATE, EXPERT] {
            assert_eq!(Level::from_code(&level.code()), Some(level));
        }
        assert_eq!(BEGINNER.code(), "beginner");
    }

    #[test]
    fn a_custom_board_round_trips_and_a_bad_code_is_refused() {
        let level = custom(20, 12, 30).unwrap();
        assert_eq!(level.code(), "20x12x30");
        assert_eq!(Level::from_code("20x12x30"), Some(level));
        for bad in ["", "x", "20x12", "20x12x30x1", "4x9x10", "31x9x10", "9x25x10", "9x9x0", "9x9x29", "axbxc"] {
            assert_eq!(Level::from_code(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_custom_board_may_be_a_third_mines_and_no_more() {
        assert!(custom(10, 10, 35).is_some());
        assert!(custom(10, 10, 36).is_none());
    }

    #[test]
    fn typed_sizes_are_read_in_the_usual_spellings() {
        let want = custom(20, 12, 30).unwrap();
        for text in ["20x12 30", "20 12 30", "20,12,30", " 20 X 12 30 "] {
            assert_eq!(parse_custom(text), Ok(want), "{text}");
        }
    }

    #[test]
    fn typed_nonsense_says_what_to_type() {
        assert!(parse_custom("big").unwrap_err().contains("not a number"));
        assert!(parse_custom("20x12").unwrap_err().contains("20x12 30"));
        assert!(parse_custom("99x99 5").unwrap_err().contains("columns"));
    }

    #[test]
    fn only_the_three_names_are_presets() {
        assert_eq!(Level::from_preset("expert"), Some(EXPERT));
        assert_eq!(Level::from_preset("custom"), None);
        assert_eq!(Level::from_preset("20x12x30"), None);
    }
}
