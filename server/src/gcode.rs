//! Single-pass G-code interpreter.
//!
//! Produces the typed segment list consumed by both the WebGL toolpath viewer
//! and the simulator's motion integrator. Coordinates are resolved to machine
//! space at parse time using a [`Context`] snapshot (work offsets, G92, tool
//! lengths), exactly like a controller's interpreter would.
//!
//! Supported: G0/1/2/3 (IJK/R arcs, helical), G4, G17/18/19, G20/21,
//! G43/G49 tool length, G54–G59.3, G10 L2/L20, G80–G83 + G98/G99 canned
//! cycles, G90/91, G90.1/91.1, G92/G92.1, M0/M1 program pauses, M2/30,
//! M3/4/5 + S spindle, M6 + T tool change, M7/8/9 coolant, block delete.
//!
//! Distances are normalized to millimeters, feeds to mm/min.

use std::collections::HashMap;

use serde::Serialize;
use thiserror::Error;

/// Max chord deviation when tessellating arcs, in mm.
const ARC_TOLERANCE_MM: f64 = 0.05;

pub type Point = [f64; 3];

pub const WCS_NAMES: [&str; 9] = ["G54", "G55", "G56", "G57", "G58", "G59", "G59.1", "G59.2", "G59.3"];

/// Machine snapshot the interpreter resolves coordinates against.
#[derive(Debug, Clone)]
pub struct Context {
    /// G54..G59.3 origin offsets, machine coords.
    pub wcs: [[f64; 3]; 9],
    /// Active system at program start (0 = G54).
    pub active_wcs: usize,
    pub g92: [f64; 3],
    /// Tool number → length offset (Z).
    pub tool_lengths: HashMap<u16, f64>,
    pub current_tool: u16,
    /// Skip lines starting with '/' when true.
    pub block_delete: bool,
}

impl Default for Context {
    fn default() -> Self {
        Self {
            wcs: [[0.0; 3]; 9],
            active_wcs: 0,
            g92: [0.0; 3],
            tool_lengths: HashMap::new(),
            current_tool: 0,
            block_delete: false,
        }
    }
}

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
    /// Polyline in machine coordinates; first point is the start position.
    pub points: Vec<Point>,
    /// Programmed feed in mm/min (0 for rapids — resolved by the machine).
    #[serde(skip)]
    pub feed: f64,
    /// 1-based source line number.
    pub line: usize,
    #[serde(skip)]
    pub length: f64,
    /// Spindle speed programmed for this segment (0 = spindle off).
    #[serde(skip)]
    pub rpm: f64,
    #[serde(skip)]
    pub coolant: bool,
    /// Active tool number.
    #[serde(skip)]
    pub tool: u16,
}

impl Segment {
    fn finish(mut self) -> Self {
        self.length = self.points.windows(2).map(|w| dist(w[0], w[1])).sum();
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PauseKind {
    /// M0 — unconditional program stop.
    Mandatory,
    /// M1 — stops only when the operator armed optional stop.
    Optional,
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
    /// (segment index, kind): pause *before* executing that segment index
    /// (index == segments.len() means pause at end of program).
    #[serde(skip)]
    pub pauses: Vec<(usize, PauseKind)>,
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
    ctx: Context,
    /// Machine-coordinate position.
    pos: Point,
    motion: u8,
    plane: u8,
    metric: bool,
    absolute: bool,
    arc_absolute: bool,
    feed: f64,
    /// Spindle: programmed speed and on/off.
    rpm_word: f64,
    spindle_on: bool,
    coolant: bool,
    pending_tool: u16,
    /// Tool length compensation currently applied to Z (G43/G49).
    tool_comp: f64,
    /// Canned cycle modal state.
    cycle: Option<u8>,
    cycle_r: f64,
    cycle_z: f64,
    cycle_q: f64,
    /// G98 retracts to the Z where the cycle series started; G99 to R.
    retract_initial: bool,
    cycle_initial_z: f64,
}

impl Interp {
    fn new(ctx: Context) -> Self {
        Self {
            ctx,
            pos: [0.0; 3],
            motion: 0,
            plane: 17,
            metric: true,
            absolute: true,
            arc_absolute: false,
            feed: 0.0,
            rpm_word: 0.0,
            spindle_on: false,
            coolant: false,
            pending_tool: 0,
            tool_comp: 0.0,
            cycle: None,
            cycle_r: 0.0,
            cycle_z: 0.0,
            cycle_q: 0.0,
            retract_initial: false,
            cycle_initial_z: 0.0,
        }
    }

    fn scale(&self, v: f64) -> f64 {
        if self.metric { v } else { v * 25.4 }
    }

    /// Total work→machine offset for an axis right now.
    fn offset(&self, i: usize) -> f64 {
        let mut o = self.ctx.wcs[self.ctx.active_wcs][i] + self.ctx.g92[i];
        if i == 2 {
            o += self.tool_comp;
        }
        o
    }

    fn segment(&self, kind: SegmentKind, points: Vec<Point>, lineno: usize) -> Segment {
        Segment {
            kind,
            points,
            feed: if kind == SegmentKind::Rapid { 0.0 } else { self.feed },
            line: lineno,
            length: 0.0,
            rpm: if self.spindle_on { self.rpm_word } else { 0.0 },
            coolant: self.coolant,
            tool: self.ctx.current_tool,
        }
    }
}

/// Parse against identity offsets — for callers (and tests) that don't have
/// a machine snapshot.
#[allow(dead_code)]
pub fn parse(name: &str, source: &str) -> Result<Program, GCodeError> {
    parse_with(name, source, Context::default())
}

pub fn parse_with(name: &str, source: &str, ctx: Context) -> Result<Program, GCodeError> {
    let mut interp = Interp::new(ctx);
    let mut prog = Program {
        name: name.to_string(),
        source: source.to_string(),
        segments: Vec::new(),
        total_lines: source.lines().count(),
        extent_min: [0.0; 3],
        extent_max: [0.0; 3],
        pauses: Vec::new(),
    };
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];

    'lines: for (idx, raw) in source.lines().enumerate() {
        let lineno = idx + 1;
        let text = strip_comments(raw);
        let mut text = text.trim();
        if text.starts_with('/') {
            if interp.ctx.block_delete {
                continue;
            }
            text = text[1..].trim();
        }
        if text.is_empty() || text.starts_with('%') {
            continue;
        }
        let words = lex(text, lineno)?;
        if words.is_empty() {
            continue;
        }
        let mut out = Vec::new();
        let end = interp.line(&words, lineno, &mut out, &mut prog.pauses)?;
        for seg in out {
            let seg = seg.finish();
            if seg.length > 1e-9 {
                for p in &seg.points {
                    for i in 0..3 {
                        lo[i] = lo[i].min(p[i]);
                        hi[i] = hi[i].max(p[i]);
                    }
                }
                prog.segments.push(seg);
            }
        }
        // pauses recorded on this line stop before whatever comes next
        for p in prog.pauses.iter_mut() {
            if p.0 == usize::MAX {
                p.0 = prog.segments.len();
            }
        }
        if end {
            break 'lines;
        }
    }
    if !prog.segments.is_empty() {
        prog.extent_min = lo;
        prog.extent_max = hi;
    }
    Ok(prog)
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

struct LineWords {
    target_work: [Option<f64>; 3],
    offsets: [Option<f64>; 3], // i, j, k (raw, scaled)
    radius: Option<f64>,
    q: Option<f64>,
    l: Option<f64>,
    p: Option<f64>,
    h: Option<u16>,
}

impl Interp {
    /// Returns true at end of program.
    fn line(
        &mut self,
        words: &[(char, f64)],
        lineno: usize,
        out: &mut Vec<Segment>,
        pauses: &mut Vec<(usize, PauseKind)>,
    ) -> Result<bool, GCodeError> {
        let mut w = LineWords {
            target_work: [None; 3],
            offsets: [None; 3],
            radius: None,
            q: None,
            l: None,
            p: None,
            h: None,
        };
        let mut motion_on_line = false;
        let mut g10 = false;
        let mut g43 = false;
        let mut g92_set = false;
        let mut tool_change = false;
        // emitted-so-far counter lives in the caller; pauses use a sentinel
        // fixed up by the caller via the running segment count
        let base = usize::MAX; // placeholder, resolved below

        for &(letter, value) in words {
            match letter {
                'g' => {
                    let g = (value * 10.0).round() / 10.0;
                    match g {
                        x if x == 0.0 || x == 1.0 || x == 2.0 || x == 3.0 => {
                            self.motion = g as u8;
                            self.cycle = None;
                            motion_on_line = true;
                        }
                        x if x == 4.0 => {} // dwell: no geometry
                        x if x == 10.0 => g10 = true,
                        x if x == 17.0 || x == 18.0 || x == 19.0 => self.plane = g as u8,
                        x if x == 20.0 => self.metric = false,
                        x if x == 21.0 => self.metric = true,
                        x if x == 43.0 => g43 = true,
                        x if x == 49.0 => self.tool_comp = 0.0,
                        x if (54.0..=59.0).contains(&x) && x.fract() == 0.0 => {
                            self.ctx.active_wcs = (g as usize) - 54;
                        }
                        x if x == 59.1 => self.ctx.active_wcs = 6,
                        x if x == 59.2 => self.ctx.active_wcs = 7,
                        x if x == 59.3 => self.ctx.active_wcs = 8,
                        x if x == 80.0 => self.cycle = None,
                        x if (81.0..=83.0).contains(&x) && x.fract() == 0.0 => {
                            if self.cycle.is_none() {
                                self.cycle_initial_z = self.pos[2];
                            }
                            self.cycle = Some(g as u8);
                            motion_on_line = true;
                        }
                        x if x == 90.0 => self.absolute = true,
                        x if x == 91.0 => self.absolute = false,
                        x if x == 90.1 => self.arc_absolute = true,
                        x if x == 91.1 => self.arc_absolute = false,
                        x if x == 92.0 => g92_set = true,
                        x if x == 92.1 => self.ctx.g92 = [0.0; 3],
                        x if x == 98.0 => self.retract_initial = true,
                        x if x == 99.0 => self.retract_initial = false,
                        _ => {} // offsets/canned-cycle variants we don't visualize
                    }
                }
                'm' => match value as i64 {
                    0 => pauses.push((base, PauseKind::Mandatory)),
                    1 => pauses.push((base, PauseKind::Optional)),
                    2 | 30 => return Ok(true),
                    3 | 4 => self.spindle_on = true,
                    5 => self.spindle_on = false,
                    6 => tool_change = true,
                    7 | 8 => self.coolant = true,
                    9 => self.coolant = false,
                    _ => {}
                },
                'f' => self.feed = self.scale(value),
                's' => self.rpm_word = value,
                't' => self.pending_tool = value as u16,
                'x' | 'y' | 'z' => {
                    let i = (letter as u8 - b'x') as usize;
                    w.target_work[i] = Some(self.scale(value));
                }
                'i' | 'j' | 'k' => {
                    let i = (letter as u8 - b'i') as usize;
                    w.offsets[i] = Some(self.scale(value));
                }
                'r' => w.radius = Some(self.scale(value)),
                'q' => w.q = Some(self.scale(value)),
                'l' => w.l = Some(value),
                'p' => w.p = Some(value),
                'h' => w.h = Some(value as u16),
                _ => {} // n and friends carry no geometry
            }
        }

        if tool_change {
            self.ctx.current_tool = self.pending_tool;
        }
        if g43 {
            let tool = w.h.unwrap_or(self.ctx.current_tool);
            self.tool_comp = self.ctx.tool_lengths.get(&tool).copied().unwrap_or(0.0);
        }
        if g10 {
            self.apply_g10(&w, lineno)?;
            return Ok(false);
        }
        if g92_set {
            for i in 0..3 {
                if let Some(v) = w.target_work[i] {
                    let tool = if i == 2 { self.tool_comp } else { 0.0 };
                    self.ctx.g92[i] = self.pos[i] - self.ctx.wcs[self.ctx.active_wcs][i] - tool - v;
                }
            }
            return Ok(false);
        }

        // resolve target in machine coordinates
        let mut target = self.pos;
        let mut has_axis_word = false;
        for i in 0..3 {
            if let Some(v) = w.target_work[i] {
                has_axis_word = true;
                target[i] = if self.absolute { v + self.offset(i) } else { self.pos[i] + v };
            }
        }

        if let Some(cycle) = self.cycle {
            if has_axis_word || motion_on_line {
                self.canned_cycle(cycle, &w, target, lineno, out)?;
            }
            return Ok(false);
        }

        let arc_words = w.offsets.iter().any(Option::is_some) || w.radius.is_some();
        if !has_axis_word && !(motion_on_line && (self.motion == 2 || self.motion == 3) && arc_words) {
            return Ok(false);
        }

        match self.motion {
            0 | 1 => {
                let kind = if self.motion == 0 { SegmentKind::Rapid } else { SegmentKind::Feed };
                out.push(self.segment(kind, vec![self.pos, target], lineno));
                self.pos = target;
            }
            2 | 3 => {
                let points = self.arc(target, &w.offsets, w.radius, self.motion == 2, lineno)?;
                out.push(self.segment(SegmentKind::Arc, points, lineno));
                self.pos = target;
            }
            _ => {}
        }
        Ok(false)
    }

    fn apply_g10(&mut self, w: &LineWords, lineno: usize) -> Result<(), GCodeError> {
        let l = w.l.unwrap_or(0.0) as i64;
        let p = w.p.unwrap_or(0.0) as i64;
        if !(1..=9).contains(&p) {
            return Err(err(lineno, format!("G10 P{p} out of range (1–9)")));
        }
        let sys = (p - 1) as usize;
        match l {
            2 => {
                for i in 0..3 {
                    if let Some(v) = w.target_work[i] {
                        self.ctx.wcs[sys][i] = v;
                    }
                }
            }
            20 => {
                for i in 0..3 {
                    if let Some(v) = w.target_work[i] {
                        let tool = if i == 2 { self.tool_comp } else { 0.0 };
                        self.ctx.wcs[sys][i] = self.pos[i] - self.ctx.g92[i] - tool - v;
                    }
                }
            }
            other => return Err(err(lineno, format!("unsupported G10 L{other}"))),
        }
        Ok(())
    }

    /// G81 (drill), G82 (drill + dwell), G83 (peck) at the line's XY.
    fn canned_cycle(
        &mut self,
        cycle: u8,
        w: &LineWords,
        target: Point,
        lineno: usize,
        out: &mut Vec<Segment>,
    ) -> Result<(), GCodeError> {
        if let Some(r) = w.radius {
            self.cycle_r = if self.absolute { r + self.offset(2) } else { self.pos[2] + r };
        }
        if let Some(z) = w.target_work[2] {
            self.cycle_z = if self.absolute { z + self.offset(2) } else { self.cycle_r + z };
        }
        if let Some(q) = w.q {
            self.cycle_q = q.abs();
        }
        if self.cycle_z >= self.cycle_r {
            return Err(err(lineno, "canned cycle Z must be below R"));
        }

        let (x, y) = (target[0], target[1]);
        let mut emit = |s: &mut Self, kind, to: Point, line| {
            let seg = s.segment(kind, vec![s.pos, to], line);
            out.push(seg);
            s.pos = to;
        };

        // rapid to XY at current height, then to R plane
        emit(self, SegmentKind::Rapid, [x, y, self.pos[2]], lineno);
        if self.pos[2] != self.cycle_r {
            emit(self, SegmentKind::Rapid, [x, y, self.cycle_r], lineno);
        }

        match cycle {
            81 | 82 => {
                emit(self, SegmentKind::Feed, [x, y, self.cycle_z], lineno);
            }
            83 => {
                let q = if self.cycle_q > 1e-9 { self.cycle_q } else { self.cycle_r - self.cycle_z };
                let mut depth = self.cycle_r;
                while depth > self.cycle_z + 1e-9 {
                    let next = (depth - q).max(self.cycle_z);
                    emit(self, SegmentKind::Feed, [x, y, next], lineno);
                    if next > self.cycle_z + 1e-9 {
                        emit(self, SegmentKind::Rapid, [x, y, self.cycle_r], lineno);
                        emit(self, SegmentKind::Rapid, [x, y, next + 0.5], lineno);
                    }
                    depth = next;
                }
            }
            _ => return Err(err(lineno, format!("unsupported cycle G{cycle}"))),
        }

        let retract_z = if self.retract_initial { self.cycle_initial_z } else { self.cycle_r };
        emit(self, SegmentKind::Rapid, [x, y, retract_z], lineno);
        Ok(())
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
                (ou + self.offset(ui), ov + self.offset(vi))
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

    #[test]
    fn wcs_offsets_resolve_to_machine_coords() {
        let mut ctx = Context::default();
        ctx.wcs[0] = [100.0, 50.0, -10.0]; // G54
        ctx.wcs[1] = [200.0, 0.0, 0.0]; // G55
        let p = parse_with("t", "G0 X0 Y0 Z0\nG55 G0 X1 Y0 Z0", ctx).unwrap();
        assert_eq!(p.segments[0].points[1], [100.0, 50.0, -10.0]);
        assert_eq!(p.segments[1].points[1], [201.0, 0.0, 0.0]);
    }

    #[test]
    fn g92_shifts_work_coords() {
        // at machine X10, G92 X0 declares "here is work 0" → next X5 lands at machine 15
        let p = parse("t", "G0 X10\nG92 X0\nG0 X5").unwrap();
        assert_eq!(p.segments[1].points[1][0], 15.0);
    }

    #[test]
    fn g10_l2_sets_offsets() {
        let p = parse("t", "G10 L2 P2 X50\nG55 G0 X0").unwrap();
        assert_eq!(p.segments[0].points[1][0], 50.0);
    }

    #[test]
    fn tool_length_applies_to_z() {
        let mut ctx = Context::default();
        ctx.tool_lengths.insert(3, 25.0);
        let p = parse_with("t", "T3 M6 G43 H3\nG0 Z0\nG49\nG0 Z0", ctx).unwrap();
        assert_eq!(p.segments[0].points[1][2], 25.0); // compensated
        assert_eq!(p.segments[0].tool, 3);
        assert_eq!(p.segments[1].points[1][2], 0.0); // G49: raw
    }

    #[test]
    fn spindle_and_coolant_tracked() {
        let p = parse("t", "M3 S9000 M8\nG1 X10 F500\nM5 M9\nG1 X20").unwrap();
        assert_eq!(p.segments[0].rpm, 9000.0);
        assert!(p.segments[0].coolant);
        assert_eq!(p.segments[1].rpm, 0.0);
        assert!(!p.segments[1].coolant);
    }

    #[test]
    fn program_pauses() {
        let p = parse("t", "G0 X10\nM0\nG0 X20\nM1\n").unwrap();
        assert_eq!(p.pauses, vec![(1, PauseKind::Mandatory), (2, PauseKind::Optional)]);
    }

    #[test]
    fn block_delete_skips_slash_lines() {
        let mut ctx = Context::default();
        ctx.block_delete = true;
        let p = parse_with("t", "G0 X10\n/G0 X99\nG0 X20", ctx).unwrap();
        assert_eq!(p.segments.len(), 2);
        let p2 = parse("t", "G0 X10\n/G0 X99\nG0 X20").unwrap();
        assert_eq!(p2.segments.len(), 3); // switch off: slash lines run
    }

    #[test]
    fn canned_cycle_g81() {
        let p = parse("t", "G0 X0 Y0 Z5\nG99 G81 X10 Y0 R1 Z-4 F300\nX20\nG80").unwrap();
        // per hole: rapid XY, rapid to R, feed to Z, rapid back to R
        let feeds: Vec<_> = p.segments.iter().filter(|s| s.kind == SegmentKind::Feed).collect();
        assert_eq!(feeds.len(), 2);
        assert_eq!(feeds[0].points[1], [10.0, 0.0, -4.0]);
        assert_eq!(feeds[1].points[1], [20.0, 0.0, -4.0]);
        // G99: retract to R plane
        assert_eq!(p.segments.last().unwrap().points[1][2], 1.0);
    }

    #[test]
    fn canned_cycle_g83_pecks() {
        let p = parse("t", "G0 Z5\nG99 G83 X0 Y0 R0 Z-10 Q4 F200\nG80").unwrap();
        let feeds: Vec<_> = p.segments.iter().filter(|s| s.kind == SegmentKind::Feed).collect();
        // pecks: -4, -8, -10
        assert_eq!(feeds.len(), 3);
        assert_eq!(feeds.last().unwrap().points[1][2], -10.0);
    }

    #[test]
    fn g98_retracts_to_initial_height() {
        let p = parse("t", "G0 X0 Y0 Z7\nG98 G81 X10 R1 Z-2 F300\nG80").unwrap();
        assert_eq!(p.segments.last().unwrap().points[1][2], 7.0);
    }
}
