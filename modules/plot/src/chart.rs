//! Turns a list of numbers into a bar-chart scene. A worked example of a
//! module whose output is a `scene/1` drawing rather than text - slim paints
//! whatever the command returns, so "render a chart" is just "return a scene".

use serde_json::{json, Value};

/// The most bars we draw; past this a chart is unreadable at message size, so
/// extra values are dropped rather than crammed in.
const MAX_BARS: usize = 24;

pub fn apply(command: &str, input: &str) -> Result<String, String> {
    match command {
        "bars" => bars(input),
        other => Err(format!("unknown command: {other}")),
    }
}

fn bars(input: &str) -> Result<String, String> {
    let data = parse(input);
    if data.is_empty() {
        return Err(
            "give me some numbers - `3, 7, 2, 8` or `mon: 3, tue: 7` (one per line or comma-separated)"
                .to_string(),
        );
    }
    Ok(render(&data))
}

/// Parses labelled or bare numbers, split on newlines and commas. Each item is
/// `label: value`, `label = value`, `label value`, or a bare number (labelled
/// by its position). Non-numeric items are skipped rather than failing the lot.
fn parse(input: &str) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    for raw in input.split(['\n', ',']) {
        let item = raw.trim();
        if item.is_empty() {
            continue;
        }
        let (label, value) = split_item(item);
        if let Some(v) = value {
            let label = label.unwrap_or_else(|| (out.len() + 1).to_string());
            out.push((label, v));
            if out.len() >= MAX_BARS {
                break;
            }
        }
    }
    out
}

/// A number, but never an infinity or NaN: those would put null coordinates in the scene.
fn finite(text: &str) -> Result<f64, ()> {
    match text.parse::<f64>() {
        Ok(v) if v.is_finite() => Ok(v),
        _ => Err(()),
    }
}

/// Splits one item into an optional label and its value. A bare number has no
/// label; `label: value` / `label = value` / `label value` do.
fn split_item(item: &str) -> (Option<String>, Option<f64>) {
    if let Ok(v) = finite(item) {
        return (None, Some(v));
    }
    for sep in [':', '='] {
        if let Some((label, rest)) = item.rsplit_once(sep) {
            if let Ok(v) = finite(rest.trim()) {
                return (Some(label.trim().to_string()), Some(v));
            }
        }
    }
    if let Some((label, rest)) = item.rsplit_once(char::is_whitespace) {
        if let Ok(v) = finite(rest.trim()) {
            return (Some(label.trim().to_string()), Some(v));
        }
    }
    (None, None)
}

fn render(data: &[(String, f64)]) -> String {
    let n = data.len() as f64;
    let width = (n * 16.0 + 8.0).max(100.0);
    let height = 64.0;
    let axis_y = height - 12.0;
    let top = 10.0;
    let hi = data.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max);
    let lo = data.iter().map(|(_, v)| *v).fold(0.0_f64, f64::min);
    // Room under the lowest bar for its value label, so it clears the category labels.
    let plot_bottom = if lo < 0.0 { axis_y - 5.0 } else { axis_y };
    // The zero line sits where value 0 falls; an all-zero chart (hi == lo) draws flat bars.
    let ranged = (hi - lo) > f64::EPSILON;
    let scale = if ranged { (plot_bottom - top) / (hi - lo) } else { 0.0 };
    let zero_y = if ranged { top + hi * scale } else { axis_y };
    let slot = (width - 8.0) / n;
    let bar_w = slot * 0.66;

    let mut ops: Vec<Value> = vec![json!({
        "op": "line", "x1": 4.0, "y1": zero_y, "x2": width - 4.0, "y2": zero_y,
        "stroke": "border", "sw": 0.5
    })];
    for (i, (label, value)) in data.iter().enumerate() {
        let x = 4.0 + i as f64 * slot + (slot - bar_w) / 2.0;
        let h = (value.abs() * scale).clamp(0.0, plot_bottom - top);
        let y = if *value < 0.0 { zero_y } else { zero_y - h };
        let label_y = if *value < 0.0 { y + h + 4.5 } else { y - 1.5 };
        ops.push(json!({
            "op": "rect", "x": x, "y": y, "w": bar_w, "h": h,
            "fill": "accent", "r": 1.0
        }));
        ops.push(json!({
            "op": "text", "x": x + bar_w / 2.0, "y": label_y, "s": trim_num(*value),
            "fill": "text", "align": "center", "size": 4.0
        }));
        ops.push(json!({
            "op": "text", "x": x + bar_w / 2.0, "y": axis_y + 5.0, "s": label,
            "fill": "muted", "align": "center", "size": 4.0
        }));
    }

    let scene = json!({
        "$slim": "scene/1",
        "width": width,
        "height": height,
        "background": "surface",
        "ops": ops,
        "status": format!("{} value{}", data.len(), if data.len() == 1 { "" } else { "s" }),
        "live": false,
    });
    scene.to_string()
}

/// A compact number label: integers without a trailing `.0`, others to one dp.
fn trim_num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v + 0.0)
    } else {
        format!("{v:.1}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_numbers_labelled_by_position() {
        assert_eq!(
            parse("3, 7, 2"),
            vec![("1".into(), 3.0), ("2".into(), 7.0), ("3".into(), 2.0)]
        );
    }

    #[test]
    fn parses_labelled_values_across_separators() {
        assert_eq!(
            parse("mon: 3\ntue = 7\nwed 2"),
            vec![("mon".into(), 3.0), ("tue".into(), 7.0), ("wed".into(), 2.0)]
        );
    }

    #[test]
    fn skips_junk_and_caps_the_count() {
        // Junk is dropped; a bare number is labelled by its position among the kept values, so the lone 5 is "1".
        assert_eq!(parse("x, 5, nope"), vec![("1".into(), 5.0)]);
        assert_eq!(parse(&"1,".repeat(40)).len(), MAX_BARS);
    }

    #[test]
    fn empty_input_is_an_error_not_a_panic() {
        assert!(apply("bars", "   ").is_err());
    }

    #[test]
    fn bars_returns_a_scene_with_a_bar_per_value() {
        let out = apply("bars", "3, 7, 2").unwrap();
        assert!(out.contains(r#""$slim":"scene/1""#));
        // one axis line + 3 bars, each a rect.
        assert_eq!(out.matches(r#""op":"rect""#).count(), 3);
    }

    #[test]
    fn a_label_with_a_quote_stays_valid_json() {
        // serde_json escapes it; a hand-built string would not.
        let out = apply("bars", "a\"b: 4").unwrap();
        let _: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    }

    fn scene_ops(input: &str) -> Vec<Value> {
        let out: Value = serde_json::from_str(&apply("bars", input).unwrap()).unwrap();
        out["ops"].as_array().unwrap().clone()
    }

    fn rects(ops: &[Value]) -> Vec<&Value> {
        ops.iter().filter(|o| o["op"] == "rect").collect()
    }

    #[test]
    fn a_negative_bar_hangs_below_the_zero_line_not_above_it() {
        let ops = scene_ops("a: 5, b: -5");
        let bars = rects(&ops);
        let zero = ops[0]["y1"].as_f64().unwrap();
        let (up, down) = (bars[0], bars[1]);
        assert_eq!(up["y"].as_f64().unwrap() + up["h"].as_f64().unwrap(), zero);
        assert_eq!(down["y"].as_f64().unwrap(), zero, "negative bar must start at the zero line");
        assert_eq!(up["h"], down["h"], "equal magnitudes draw equally tall");
    }

    #[test]
    fn the_zero_line_moves_up_to_make_room_for_negatives() {
        let flat = scene_ops("a: 5, b: 5");
        let mixed = scene_ops("a: 5, b: -5");
        assert!(mixed[0]["y1"].as_f64().unwrap() < flat[0]["y1"].as_f64().unwrap());
    }

    #[test]
    fn every_bar_and_label_stays_inside_the_scene() {
        for input in ["a: 5, b: -5", "a: -3, b: -9", "a: 1, b: -100"] {
            for op in scene_ops(input) {
                for key in ["y", "y1", "y2"] {
                    if let Some(y) = op[key].as_f64() {
                        assert!((0.0..=64.0).contains(&y), "{input}: {op}");
                    }
                }
                if op["op"] == "rect" {
                    let bottom = op["y"].as_f64().unwrap() + op["h"].as_f64().unwrap();
                    assert!(bottom <= 64.0, "{input}: {op}");
                }
            }
        }
    }

    #[test]
    fn an_all_negative_chart_hangs_every_bar_from_the_top_line() {
        let ops = scene_ops("a: -3, b: -9");
        let zero = ops[0]["y1"].as_f64().unwrap();
        for bar in rects(&ops) {
            assert_eq!(bar["y"].as_f64().unwrap(), zero);
        }
    }

    #[test]
    fn an_all_zero_chart_keeps_its_line_at_the_bottom() {
        let ops = scene_ops("a: 0, b: 0");
        assert_eq!(ops[0]["y1"].as_f64().unwrap(), 52.0);
        assert!(rects(&ops).iter().all(|r| r["h"] == 0.0));
    }

    #[test]
    fn a_non_finite_value_is_skipped_like_other_junk() {
        for junk in ["a: inf, b: 3", "a: 1e999, b: 3", "a: -infinity, b: 3", "a: nan, b: 3"] {
            let scene = apply("bars", junk).unwrap();
            assert!(!scene.contains("null"), "{junk}: {scene}");
            assert!(scene.contains("1 value"), "{junk}: {scene}");
        }
        assert!(apply("bars", "inf, nan").is_err());
    }

    #[test]
    fn a_subnormal_or_tiny_range_draws_flat_bars_not_null_coordinates() {
        for input in ["1e-320", "1e-320, 2e-320", "a: 1e-300, b: 3e-300", "-1e-320"] {
            let scene = apply("bars", input).unwrap();
            assert!(!scene.contains("null"), "{input}: {scene}");
        }
    }

    #[test]
    fn a_huge_range_stays_finite() {
        let scene = apply("bars", "a: 1e308, b: -1e308").unwrap();
        assert!(!scene.contains("null"), "{scene}");
    }

    #[test]
    fn a_large_integer_label_is_not_saturated() {
        let scene = apply("bars", "a: 1e20, b: 3").unwrap();
        assert!(!scene.contains("9223372036854775807"), "{scene}");
        assert!(scene.contains("100000000000000000000"), "{scene}");
    }
}
