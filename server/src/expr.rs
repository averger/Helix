//! RS274NGC expressions and parameters.
//!
//! Evaluates `[...]` expressions with the standard precedence
//! (`**` > `* / MOD` > `+ -` > comparisons > `AND OR XOR`), unary functions
//! (SIN, COS, ATAN[y]/[x], SQRT, FIX, FUP, ROUND, EXISTS …) and parameter
//! references — numbered (`#5`, locals 1–30 inside subroutines) and named
//! (`#<gap>` local, `#<_global>` global).

use std::collections::HashMap;

pub type Result<T> = std::result::Result<T, String>;

#[derive(Default)]
pub struct Params {
    globals: HashMap<String, f64>,
    frames: Vec<HashMap<String, f64>>,
}

impl Params {
    /// Numbered params 1–30 and plain named params are subroutine-local when
    /// a call frame exists; `_`-prefixed named params are always global.
    fn is_local(&self, key: &str) -> bool {
        if self.frames.is_empty() || key.starts_with('_') {
            return false;
        }
        match key.parse::<u32>() {
            Ok(n) => (1..=30).contains(&n),
            Err(_) => true,
        }
    }

    pub fn get(&self, key: &str) -> f64 {
        if self.is_local(key) {
            if let Some(v) = self.frames.last().and_then(|f| f.get(key)) {
                return *v;
            }
            return 0.0;
        }
        self.globals.get(key).copied().unwrap_or(0.0)
    }

    pub fn exists(&self, key: &str) -> bool {
        if self.is_local(key) {
            return self.frames.last().is_some_and(|f| f.contains_key(key));
        }
        self.globals.contains_key(key)
    }

    pub fn set(&mut self, key: &str, value: f64) {
        if self.is_local(key) {
            self.frames.last_mut().unwrap().insert(key.to_string(), value);
        } else {
            self.globals.insert(key.to_string(), value);
        }
    }

    pub fn push_frame(&mut self, args: Vec<f64>) {
        let mut frame = HashMap::new();
        for (i, v) in args.into_iter().enumerate() {
            frame.insert((i + 1).to_string(), v);
        }
        self.frames.push(frame);
    }

    pub fn pop_frame(&mut self) {
        self.frames.pop();
    }
}

pub struct Cursor {
    chars: Vec<char>,
    pub pos: usize,
}

impl Cursor {
    pub fn new(text: &str) -> Self {
        Self { chars: text.chars().collect(), pos: 0 }
    }

    pub fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    pub fn next(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    pub fn skip_ws(&mut self) {
        while self.peek().is_some_and(|c| c.is_whitespace()) {
            self.pos += 1;
        }
    }

    pub fn at_end(&mut self) -> bool {
        self.skip_ws();
        self.pos >= self.chars.len()
    }

    fn starts_with_kw(&mut self, kw: &str) -> bool {
        self.skip_ws();
        let rest: String = self.chars[self.pos..].iter().take(kw.len()).collect();
        if rest.eq_ignore_ascii_case(kw) {
            // keywords must not run into an identifier
            let after = self.chars.get(self.pos + kw.len());
            if kw.chars().last().unwrap().is_ascii_alphabetic()
                && after.is_some_and(|c| c.is_ascii_alphanumeric())
            {
                return false;
            }
            self.pos += kw.len();
            return true;
        }
        false
    }

    fn number(&mut self) -> Result<f64> {
        self.skip_ws();
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.') {
            self.pos += 1;
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        text.parse().map_err(|_| format!("bad number {text:?}"))
    }
}

/// `#5`, `#<name>` or `#[expr]` — returns the parameter key.
pub fn param_key(cur: &mut Cursor, params: &Params) -> Result<String> {
    cur.skip_ws();
    match cur.peek() {
        Some('<') => {
            cur.next();
            let mut name = String::new();
            while let Some(c) = cur.next() {
                if c == '>' {
                    return Ok(name.to_lowercase());
                }
                if !c.is_whitespace() {
                    name.push(c);
                }
            }
            Err("unterminated #<name>".into())
        }
        Some('[') => {
            let n = eval(cur, params)?;
            Ok(format!("{}", n.round() as i64))
        }
        _ => {
            let n = cur.number()?;
            Ok(format!("{}", n.round() as i64))
        }
    }
}

/// A "real value": number, #param, [expr], unary function, or unary minus.
pub fn value(cur: &mut Cursor, params: &Params) -> Result<f64> {
    cur.skip_ws();
    match cur.peek() {
        None => Err("expected a value".into()),
        Some('-') => {
            cur.next();
            Ok(-value(cur, params)?)
        }
        Some('+') => {
            cur.next();
            value(cur, params)
        }
        Some('#') => {
            cur.next();
            let key = param_key(cur, params)?;
            Ok(params.get(&key))
        }
        Some('[') => eval(cur, params),
        Some(c) if c.is_ascii_digit() || c == '.' => cur.number(),
        Some(_) => unary(cur, params),
    }
}

fn unary(cur: &mut Cursor, params: &Params) -> Result<f64> {
    let mut name = String::new();
    while cur.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
        name.push(cur.next().unwrap());
    }
    let name = name.to_uppercase();
    if name == "EXISTS" {
        cur.skip_ws();
        if cur.next() != Some('[') {
            return Err("EXISTS expects [#<name>]".into());
        }
        cur.skip_ws();
        if cur.next() != Some('#') {
            return Err("EXISTS expects [#<name>]".into());
        }
        let key = param_key(cur, params)?;
        cur.skip_ws();
        if cur.next() != Some(']') {
            return Err("unterminated EXISTS".into());
        }
        return Ok(if params.exists(&key) { 1.0 } else { 0.0 });
    }
    let arg = eval(cur, params)?;
    let deg = std::f64::consts::PI / 180.0;
    match name.as_str() {
        "ABS" => Ok(arg.abs()),
        "ACOS" => Ok(arg.acos() / deg),
        "ASIN" => Ok(arg.asin() / deg),
        "ATAN" => {
            cur.skip_ws();
            if cur.next() != Some('/') {
                return Err("ATAN expects [y]/[x]".into());
            }
            let x = eval(cur, params)?;
            Ok(arg.atan2(x) / deg)
        }
        "COS" => Ok((arg * deg).cos()),
        "EXP" => Ok(arg.exp()),
        "FIX" => Ok(arg.floor()),
        "FUP" => Ok(arg.ceil()),
        "LN" => Ok(arg.ln()),
        "ROUND" => Ok(arg.round()),
        "SIN" => Ok((arg * deg).sin()),
        "SQRT" => Ok(arg.sqrt()),
        "TAN" => Ok((arg * deg).tan()),
        other => Err(format!("unknown function {other}")),
    }
}

/// Evaluate a bracketed expression; the cursor must sit on `[`.
pub fn eval(cur: &mut Cursor, params: &Params) -> Result<f64> {
    cur.skip_ws();
    if cur.next() != Some('[') {
        return Err("expected [".into());
    }
    let v = logical(cur, params)?;
    cur.skip_ws();
    if cur.next() != Some(']') {
        return Err("unterminated expression".into());
    }
    Ok(v)
}

fn logical(cur: &mut Cursor, params: &Params) -> Result<f64> {
    let mut left = comparison(cur, params)?;
    loop {
        if cur.starts_with_kw("AND") {
            let r = comparison(cur, params)?;
            left = f64::from(left != 0.0 && r != 0.0);
        } else if cur.starts_with_kw("XOR") {
            let r = comparison(cur, params)?;
            left = f64::from((left != 0.0) != (r != 0.0));
        } else if cur.starts_with_kw("OR") {
            let r = comparison(cur, params)?;
            left = f64::from(left != 0.0 || r != 0.0);
        } else {
            return Ok(left);
        }
    }
}

fn comparison(cur: &mut Cursor, params: &Params) -> Result<f64> {
    let left = additive(cur, params)?;
    const EPS: f64 = 1e-9;
    for (kw, f) in [
        ("EQ", (|a: f64, b: f64| (a - b).abs() < EPS) as fn(f64, f64) -> bool),
        ("NE", |a, b| (a - b).abs() >= EPS),
        ("GE", |a, b| a >= b - EPS),
        ("GT", |a, b| a > b + EPS),
        ("LE", |a, b| a <= b + EPS),
        ("LT", |a, b| a < b - EPS),
    ] {
        if cur.starts_with_kw(kw) {
            let right = additive(cur, params)?;
            return Ok(f64::from(f(left, right)));
        }
    }
    Ok(left)
}

fn additive(cur: &mut Cursor, params: &Params) -> Result<f64> {
    let mut left = multiplicative(cur, params)?;
    loop {
        cur.skip_ws();
        match cur.peek() {
            Some('+') => {
                cur.next();
                left += multiplicative(cur, params)?;
            }
            Some('-') => {
                cur.next();
                left -= multiplicative(cur, params)?;
            }
            _ => return Ok(left),
        }
    }
}

fn multiplicative(cur: &mut Cursor, params: &Params) -> Result<f64> {
    let mut left = power(cur, params)?;
    loop {
        cur.skip_ws();
        if cur.starts_with_kw("MOD") {
            let r = power(cur, params)?;
            left = left.rem_euclid(r);
            continue;
        }
        match cur.peek() {
            Some('*') if cur.chars.get(cur.pos + 1) != Some(&'*') => {
                cur.next();
                left *= power(cur, params)?;
            }
            Some('/') => {
                cur.next();
                left /= power(cur, params)?;
            }
            _ => return Ok(left),
        }
    }
}

fn power(cur: &mut Cursor, params: &Params) -> Result<f64> {
    let base = value(cur, params)?;
    cur.skip_ws();
    if cur.peek() == Some('*') && cur.chars.get(cur.pos + 1) == Some(&'*') {
        cur.pos += 2;
        let exp = power(cur, params)?; // right-associative
        return Ok(base.powf(exp));
    }
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(text: &str) -> f64 {
        eval(&mut Cursor::new(text), &Params::default()).unwrap()
    }

    #[test]
    fn precedence() {
        assert_eq!(ev("[1 + 2 * 3]"), 7.0);
        assert_eq!(ev("[2 ** 3 * 2]"), 16.0);
        assert_eq!(ev("[[1 + 2] * 3]"), 9.0);
        assert_eq!(ev("[10 MOD 3]"), 1.0);
    }

    #[test]
    fn comparisons_and_logic() {
        assert_eq!(ev("[3 GT 2]"), 1.0);
        assert_eq!(ev("[1 EQ 2]"), 0.0);
        assert_eq!(ev("[[3 GT 2] AND [1 LT 2]]"), 1.0);
        assert_eq!(ev("[0 OR 1]"), 1.0);
    }

    #[test]
    fn functions_use_degrees() {
        assert!((ev("[SIN[30]]") - 0.5).abs() < 1e-9);
        assert!((ev("[ATAN[1]/[1]]") - 45.0).abs() < 1e-9);
        assert_eq!(ev("[SQRT[16]]"), 4.0);
        assert_eq!(ev("[FIX[2.8]]"), 2.0);
        assert_eq!(ev("[FUP[2.2]]"), 3.0);
    }

    #[test]
    fn parameters_and_scoping() {
        let mut p = Params::default();
        p.set("5", 12.0);
        p.set("_name", 3.0);
        assert_eq!(eval(&mut Cursor::new("[#5 + #<_name>]"), &p).unwrap(), 15.0);
        // locals shadow inside a frame
        p.push_frame(vec![100.0]);
        assert_eq!(p.get("1"), 100.0);
        p.set("1", 101.0);
        p.pop_frame();
        assert_eq!(p.get("1"), 0.0); // local vanished with the frame
        assert_eq!(p.get("5"), 12.0); // >30 stays global
    }
}
