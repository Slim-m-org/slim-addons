//! A small recursive-descent evaluator: + - * / % ^, unary minus, names, calls.

use std::collections::BTreeMap;

const MAX_DEPTH: usize = 64;
const FUNCTIONS: [&str; 10] = ["sqrt", "abs", "round", "floor", "ceil", "min", "max", "ln", "log10", "sign"];

pub fn is_reserved(name: &str) -> bool {
    name == "pi" || name == "e" || FUNCTIONS.contains(&name)
}

fn finite(value: f64) -> Result<f64, String> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err("the result is not a finite number".to_string())
    }
}

pub fn evaluate(source: &str, vars: &BTreeMap<String, f64>) -> Result<f64, String> {
    let mut p = Parser { chars: source.chars().collect(), pos: 0, depth: 0, vars };
    let value = p.sum()?;
    p.skip_space();
    match p.chars.get(p.pos) {
        None => Ok(value),
        Some(c) => Err(format!("unexpected {c:?}")),
    }
}

/// Integers print plainly; everything else keeps ten decimals, trimmed.
pub fn format_number(v: f64) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    let magnitude = v.abs();
    if !(1e-9..1e15).contains(&magnitude) {
        return format_exponent(v);
    }
    let fixed = format!("{v:.10}");
    fixed.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn format_exponent(v: f64) -> String {
    let text = format!("{v:.8e}");
    let (mantissa, exp) = text.split_once('e').unwrap_or((&text, "0"));
    let mantissa = if mantissa.contains('.') { mantissa.trim_end_matches('0').trim_end_matches('.') } else { mantissa };
    format!("{mantissa}e{exp}")
}

struct Parser<'a> {
    chars: Vec<char>,
    pos: usize,
    depth: usize,
    vars: &'a BTreeMap<String, f64>,
}

impl Parser<'_> {
    fn skip_space(&mut self) {
        while self.chars.get(self.pos).is_some_and(|c| c.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, want: char) -> bool {
        self.skip_space();
        let hit = self.chars.get(self.pos) == Some(&want);
        if hit {
            self.pos += 1;
        }
        hit
    }

    fn sum(&mut self) -> Result<f64, String> {
        let mut acc = self.product()?;
        loop {
            if self.eat('+') {
                acc = finite(acc + self.product()?)?;
            } else if self.eat('-') {
                acc = finite(acc - self.product()?)?;
            } else {
                return Ok(acc);
            }
        }
    }

    fn product(&mut self) -> Result<f64, String> {
        let mut acc = self.unary()?;
        loop {
            if self.eat('*') {
                acc = finite(acc * self.unary()?)?;
            } else if self.eat('/') {
                acc = finite(acc / self.divisor()?)?;
            } else if self.eat('%') {
                acc = finite(acc % self.divisor()?)?;
            } else {
                return Ok(acc);
            }
        }
    }

    fn divisor(&mut self) -> Result<f64, String> {
        match self.unary()? {
            d if d == 0.0 => Err("division by zero".to_string()),
            d => Ok(d),
        }
    }

    fn unary(&mut self) -> Result<f64, String> {
        if self.eat('-') {
            return Ok(-self.unary()?);
        }
        if self.eat('+') {
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Result<f64, String> {
        let base = self.atom()?;
        if self.eat('^') {
            let exponent = self.unary()?;
            return finite(pow(base, exponent));
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<f64, String> {
        self.skip_space();
        let before = self.chars.get(self.pos.wrapping_sub(1)).copied().unwrap_or(' ');
        match self.chars.get(self.pos).copied() {
            Some('(') => {
                self.pos += 1;
                self.nested(|p| p.sum().and_then(|v| p.close(v)))
            }
            Some(c) if c.is_ascii_digit() || c == '.' => self.number(),
            Some(c) if c.is_ascii_alphabetic() || c == '_' => self.name(),
            _ => Err(format!("expected a number, name or ( after {before}")),
        }
    }

    fn close(&mut self, v: f64) -> Result<f64, String> {
        if self.eat(')') {
            Ok(v)
        } else {
            Err("missing )".to_string())
        }
    }

    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        if self.depth >= MAX_DEPTH {
            return Err("too deeply nested".to_string());
        }
        self.depth += 1;
        let out = f(self);
        self.depth -= 1;
        out
    }

    fn number(&mut self) -> Result<f64, String> {
        let start = self.pos;
        while self.chars.get(self.pos).is_some_and(|c| c.is_ascii_digit() || *c == '.') {
            self.pos += 1;
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        text.parse().map_err(|_| format!("{text} is not a number")).and_then(finite)
    }

    fn name(&mut self) -> Result<f64, String> {
        let start = self.pos;
        while self.chars.get(self.pos).is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_') {
            self.pos += 1;
        }
        let name: String = self.chars[start..self.pos].iter().collect();
        if self.eat('(') {
            let args = self.nested(Self::arguments)?;
            return call(&name, &args).and_then(finite);
        }
        match name.as_str() {
            "pi" => Ok(std::f64::consts::PI),
            "e" => Ok(std::f64::consts::E),
            _ => self.vars.get(&name).copied().ok_or(format!("unknown name {name}")),
        }
    }

    fn arguments(&mut self) -> Result<Vec<f64>, String> {
        let mut args = Vec::new();
        if self.eat(')') {
            return Ok(args);
        }
        loop {
            args.push(self.sum()?);
            if self.eat(')') {
                return Ok(args);
            }
            if !self.eat(',') {
                return Err("expected , or ) in the call".to_string());
            }
        }
    }
}

fn pow(base: f64, exponent: f64) -> f64 {
    if exponent.fract() == 0.0 && exponent.abs() <= 1024.0 {
        base.powi(exponent as i32)
    } else {
        base.powf(exponent)
    }
}

fn call(name: &str, args: &[f64]) -> Result<f64, String> {
    if !FUNCTIONS.contains(&name) {
        return Err(format!("unknown function {name}"));
    }
    let one = |f: fn(f64) -> f64| match args {
        [x] => Ok(f(*x)),
        _ => Err(format!("{name} takes one argument")),
    };
    match name {
        "sqrt" if args.first().is_some_and(|x| *x < 0.0) => Err("sqrt of a negative number".to_string()),
        "ln" | "log10" if args.first().is_some_and(|x| *x <= 0.0) => Err(format!("{name} needs a positive number")),
        "sqrt" => one(f64::sqrt),
        "abs" => one(f64::abs),
        "round" => one(f64::round),
        "floor" => one(f64::floor),
        "ceil" => one(f64::ceil),
        "ln" => one(f64::ln),
        "log10" => one(f64::log10),
        "sign" => one(|x| if x == 0.0 { 0.0 } else { x.signum() }),
        _ if args.is_empty() => Err(format!("{name} needs at least one argument")),
        "min" => Ok(args.iter().copied().fold(f64::INFINITY, f64::min)),
        _ => Ok(args.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
    }
}
