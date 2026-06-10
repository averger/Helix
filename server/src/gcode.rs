//! Single-pass G-code interpreter.
//!
//! Produces the typed segment list consumed by both the WebGL toolpath viewer
//! and the simulator's motion integrator. Supports the modal subset that
//! matters for visualization and simulation: G0/1/2/3, plane select, units,
//! abs/rel, arcs by IJK or R, feed/speed words, M2/30 end of program.
//!
//! Distances are normalized to millimeters, feeds to mm/min.

use serde::Serialize;
use thiserror::Error;

/// Max chord deviation when tessellating arcs, in mm.
const ARC_TOLERANCE_MM: f64 = 0.05;

pub type Point = [f64; 3];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SegmentKind {
    Rapid,
    Feed,
    Arc,
}

#[derive(Debug, Clone, Serialize)]
pub struct Segment {
    pub kind: SegmentKind,
    /// Polyline; first point is the start position.
    pub points: Vec<Point>,
    /// Programmed feed in mm/min (0 for rapids — resolved by the machine).
    #[serde(skip)]
    pub feed: f64,
    /// 1-based source line number.
    pub line: usize,
    #[serde(skip)]
    pub length: f64,
}

impl Segment {
    fn finish(mut self) -> Self {
        self.length = self
            .points
            .windows(2)
            .map(|w| dist(w[0], w[1]))
            .sum();
        self
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Program {
    pub name: String,
    #[serde(skip)]
    pub source: String,
    pub segments: Vec<Segment>,
    pub total_lines: usize,
    pub extent_min: Point,
    pub extent_max: Point,
}

impl Program {
    pub fn length(&self) -> f64 {
        self.segments.iter().map(|s| s.length).sum()
    }
}

#[derive(Debug, Error)]
#[error("line {line}: {message}")]
pub struct GCodeError {
    pub line: usize,
    pub message: String,
}

fn err(line: usize, message: impl Into<String>) -> GCodeError {
    GCodeError { line, message: message.into() }
}

fn dist(a: Point, b: Point) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// (u, v, w) axis indices for G17/G18/G19.
fn plane_axes(plane: u8) -> (usize, usize, usize) {
    match plane {
        18 => (2, 0, 1), // ZX
        19 => (1, 2, 0), // YZ
        _ => (0, 1, 2),  // XY
    }
}

struct Interp {
    pos: Point,
    motion: u8,
    plane: u8,
    metric: bool,
    absolute: bool,
    arc_absolute: bool,
    feed: f64,
}

impl Default for Interp {
    fn default() -> Self {
        Self { pos: [0.0; 3], motion: 0, plane: 17, metric: true, absolute: true, arc_absolute: false, feed: 0.0 }
    }
}

pub fn parse(name: &str, source: &str) -> Result<Program, GCodeError> {
    let mut interp = Interp::default();
    let mut segments = Vec::new();
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    let total_lines = source.lines().count();

    'lines: for (idx, raw) in source.lines().enumerate() {
        let lineno = idx + 1;
        let text = strip_comments(raw);
        let text = text.trim();
        if text.is_empty() || text.starts_with('%') {
            continue;
        }
        let words = lex(text, lineno)?;
        if words.is_empty() {
            continue;
        }
        match interp.line(&words, lineno)? {
            LineResult::Segment(seg) => {
                let seg = seg.finish();
                if seg.length > 1e-9 {
                    for p in &seg.points {
                        for i in 0..3 {
                            lo[i] = lo[i].min(p[i]);
                            hi[i] = hi[i].max(p[i]);
                        }
                    }
                    segments.push(seg);
                }
            }
            LineResult::EndOfProgram => break 'lines,
            LineResult::Nothing => {}
        }
    }

    let (extent_min, extent_max) = if segments.is_empty() { ([0.0; 3], [0.0; 3]) } else { (lo, hi) };
    Ok(Program { name: name.to_string(), source: source.to_string(), segments, total_lines, extent_min, extent_max })
}

fn strip_comments(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut depth = 0u32;
    for c in line.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            ';' if depth == 0 => break,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

fn lex(text: &str, lineno: usize) -> Result<Vec<(char, f64)>, GCodeError> {
    let mut words = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        if !c.is_ascii_alphabetic() {
            return Err(err(lineno, format!("unexpected character {c:?}")));
        }
        let mut num = String::new();
        while let Some(&n) = chars.peek() {
            if n.is_ascii_digit() || n == '.' || n == '-' || n == '+' {
                num.push(n);
                chars.next();
            } else if n.is_whitespace() {
                if num.is_empty() {
                    chars.next();
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        let value: f64 = num
            .parse()
            .map_err(|_| err(lineno, format!("bad number {num:?} after {c:?}")))?;
        words.push((c.to_ascii_lowercase(), value));
    }
    Ok(words)
}

enum LineResult {
    Segment(Segment),
    EndOfProgram,
    Nothing,
}

impl Interp {
    fn scale(&self, v: f64) -> f64 {
        if self.metric { v } else { v * 25.4 }
    }

    fn line(&mut self, words: &[(char, f64)], lineno: usize) -> Result<LineResult, GCodeError> {
        let mut target = self.pos;
        let mut offsets = [None::<f64>; 3]; // i, j, k
        let mut radius = None::<f64>;
        let mut has_axis_word = false;
        let mut motion_on_line = false;

        for &(letter, value) in words {
            match letter {
                'g' => {
                    let g = (value * 10.0).round() / 10.0;
                    match g {
                        x if x == 0.0 || x == 1.0 || x == 2.0 || x == 3.0 => {
                            self.motion = g as u8;
                            motion_on_line = true;
                        }
                        x if x == 17.0 || x == 18.0 || x == 19.0 => self.plane = g as u8,
                        x if x == 20.0 => self.metric = false,
                        x if x == 21.0 => self.metric = true,
                        x if x == 90.0 => self.absolute = true,
                        x if x == 91.0 => self.absolute = false,
                        x if x == 90.1 => self.arc_absolute = true,
                        x if x == 91.1 => self.arc_absolute = false,
                        _ => {} // dwell, offsets, canned-cycle-off… carry no geometry
                    }
                }
                'm' => {
                    let m = value as i64;
                    if m == 2 || m == 30 {
                        return Ok(LineResult::EndOfProgram);
                    }
                    // spindle/coolant are handled by the machine layer
                }
                'f' => self.feed = self.scale(value),
                'x' | 'y' | 'z' => {
                    has_axis_word = true;
                    let i = (letter as u8 - b'x') as usize;
                    let v = self.scale(value);
                    target[i] = if self.absolute { v } else { self.pos[i] + v };
                }
                'i' | 'j' | 'k' => {
                    let i = (letter as u8 - b'i') as usize;
                    offsets[i] = Some(self.scale(value));
                }
                'r' => radius = Some(self.scale(value)),
                _ => {} // s, t, n and friends carry no geometry
            }
        }

        let arc_words = offsets.iter().any(Option::is_some) || radius.is_some();
        if !has_axis_word && !(motion_on_line && (self.motion == 2 || self.motion == 3) && arc_words) {
            return Ok(LineResult::Nothing);
        }

        let seg = match self.motion {
            0 | 1 => {
                let rapid = self.motion == 0;
                let seg = Segment {
                    kind: if rapid { SegmentKind::Rapid } else { SegmentKind::Feed },
                    points: vec![self.pos, target],
                    feed: if rapid { 0.0 } else { self.feed },
                    line: lineno,
                    length: 0.0,
                };
                self.pos = target;
                seg
            }
            2 | 3 => {
                let points = self.arc(target, &offsets, radius, self.motion == 2, lineno)?;
                let seg = Segment { kind: SegmentKind::Arc, points, feed: self.feed, line: lineno, length: 0.0 };
                self.pos = target;
                seg
            }
            _ => return Ok(LineResult::Nothing),
        };
        Ok(LineResult::Segment(seg))
    }

    fn arc(
        &self,
        target: Point,
        offsets: &[Option<f64>; 3],
        radius: Option<f64>,
        clockwise: bool,
        lineno: usize,
    ) -> Result<Vec<Point>, GCodeError> {
        let (ui, vi, wi) = plane_axes(self.plane);
        let (su, sv) = (self.pos[ui], self.pos[vi]);
        let (eu, ev) = (target[ui], target[vi]);

        let (cu, cv) = if let Some(r) = radius {
            // R-format: center on the perpendicular bisector of the chord
            let (du, dv) = (eu - su, ev - sv);
            let d = du.hypot(dv);
            if d < 1e-12 {
                return Err(err(lineno, "R-format arc with coincident endpoints"));
            }
            let h_sq = r * r - (d / 2.0).powi(2);
            if h_sq < -1e-6 {
                return Err(err(lineno, format!("arc radius {r:.4} too small for chord {d:.4}")));
            }
            let h = h_sq.max(0.0).sqrt();
            // positive R takes the minor arc
            let sign = if clockwise == (r > 0.0) { -1.0 } else { 1.0 };
            (su + du / 2.0 + sign * h * (-dv / d), sv + dv / 2.0 + sign * h * (du / d))
        } else {
            // IJK offsets follow the plane axes: u-offset, v-offset
            let ou = offsets[ui].unwrap_or(0.0);
            let ov = offsets[vi].unwrap_or(0.0);
            if self.arc_absolute {
                (ou, ov)
            } else {
                (su + ou, sv + ov)
            }
        };

        let r0 = (su - cu).hypot(sv - cv);
        if r0 < 1e-12 {
            return Err(err(lineno, "arc with zero radius"));
        }

        let a0 = (sv - cv).atan2(su - cu);
        let a1 = (ev - cv).atan2(eu - cu);
        let mut sweep = a1 - a0;
        if clockwise {
            while sweep >= -1e-9 {
                sweep -= std::f64::consts::TAU;
            }
        } else {
            while sweep <= 1e-9 {
                sweep += std::f64::consts::TAU;
            }
        }

        // chord-error-bounded tessellation
        let max_step = if r0 > ARC_TOLERANCE_MM {
            2.0 * (1.0 - ARC_TOLERANCE_MM / r0).max(0.0).acos()
        } else {
            std::f64::consts::FRAC_PI_8
        };
        let steps = ((sweep.abs() / max_step.max(1e-3)).ceil() as usize).max(2);

        let (sw, ew) = (self.pos[wi], target[wi]);
        let mut points = Vec::with_capacity(steps + 1);
        for n in 0..=steps {
            let t = n as f64 / steps as f64;
            let a = a0 + sweep * t;
            let mut p = [0.0; 3];
            p[ui] = cu + r0 * a.cos();
            p[vi] = cv + r0 * a.sin();
            p[wi] = sw + (ew - sw) * t; // helical interpolation
            points.push(p);
        }
        points[0] = self.pos;
        *points.last_mut().unwrap() = target;
        Ok(points)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_modal_motion() {
        let p = parse("t", "G21 G90\nG0 X10 Y0\nG1 Z-1 F500\nX20\nM2\nG1 X99").unwrap();
        assert_eq!(p.segments.len(), 3); // G0, G1 Z, modal G1 X20 — nothing after M2
        assert_eq!(p.segments[0].kind, SegmentKind::Rapid);
        assert_eq!(p.segments[2].points[1], [20.0, 0.0, -1.0]);
    }

    #[test]
    fn full_circle_ijk() {
        let p = parse("t", "G0 X10 Y0\nG2 X10 Y0 I-10 J0 F600").unwrap();
        let arc = &p.segments[1];
        assert_eq!(arc.kind, SegmentKind::Arc);
        // chord tessellation undershoots true arc length by < 0.5 %
        let true_len = std::f64::consts::TAU * 10.0;
        assert!((arc.length - true_len).abs() < true_len * 0.005);
    }

    #[test]
    fn helical_z() {
        let p = parse("t", "G0 X10 Y0 Z0\nG2 X10 Y0 I-10 J0 Z-3 F600").unwrap();
        let arc = &p.segments[1];
        assert_eq!(arc.points.last().unwrap()[2], -3.0);
    }

    #[test]
    fn r_format_arc() {
        // the zero-length G0 at the origin is dropped; the arc is segment 0
        let p = parse("t", "G0 X0 Y0\nG2 X10 Y0 R5 F600").unwrap();
        let arc = &p.segments[0];
        let true_len = std::f64::consts::PI * 5.0;
        assert!((arc.length - true_len).abs() < true_len * 0.005);
    }

    #[test]
    fn inches_scale() {
        let p = parse("t", "G20 G90\nG0 X1").unwrap();
        assert_eq!(p.segments[0].points[1][0], 25.4);
    }

    #[test]
    fn comments_ignored() {
        let p = parse("t", "(setup) G0 X5 ; trailing\nG1 X6 (mid) Y2 F100").unwrap();
        assert_eq!(p.segments.len(), 2);
        assert_eq!(p.segments[1].points[1], [6.0, 2.0, 0.0]);
    }
}
