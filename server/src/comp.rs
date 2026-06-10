//! Cutter radius compensation (G41/G42) — preview grade.
//!
//! Programmed moves are buffered one deep; each polyline is offset along its
//! local left normal by ± the cutter radius, consecutive moves are joined by
//! an arc around the programmed corner on outside corners and by a line-line
//! intersection (miter) on inside corners. Entry happens on the first move
//! after G41/G42, exit on G40.
//!
//! Limitations vs. a full implementation: XY plane only, no gouge detection
//! (a pocket smaller than the cutter folds instead of erroring), and
//! arc-adjacent inside corners are blended instead of trimmed. Good enough
//! to see where the cutter wall lands; the realtime core does the exact math
//! on a real machine.

use crate::gcode::{Point, Segment, SegmentKind};

const JOIN_STEP_RAD: f64 = 0.18; // ~10° per join-arc facet

struct Pending {
    seg: Segment,        // template carrying feed/rpm/line…; points = offset polyline
    prog_end: Point,
    t_end: [f64; 2],
}

#[derive(Default)]
pub struct Comp {
    /// +1 = G41 (cutter left), -1 = G42 (cutter right), 0 = off.
    side: f64,
    radius: f64,
    pending: Option<Pending>,
    last_off: Option<Point>,
}

fn norm2(v: [f64; 2]) -> Option<[f64; 2]> {
    let len = v[0].hypot(v[1]);
    if len < 1e-9 {
        return None;
    }
    Some([v[0] / len, v[1] / len])
}

/// Left normal (rotate +90°).
fn left(v: [f64; 2]) -> [f64; 2] {
    [-v[1], v[0]]
}

impl Comp {
    pub fn active(&self) -> bool {
        self.side != 0.0
    }

    pub fn engage(&mut self, left_side: bool, radius: f64) {
        self.side = if left_side { 1.0 } else { -1.0 };
        self.radius = radius.max(0.0);
    }

    pub fn disengage(&mut self, out: &mut Vec<Segment>) {
        if let Some(p) = self.pending.take() {
            // exit move: return to the programmed path at the move's end
            let mut seg = p.seg;
            seg.points.push(p.prog_end);
            self.last_off = Some(p.prog_end);
            out.push(seg);
        }
        self.side = 0.0;
        self.last_off = None;
    }

    pub fn flush(&mut self, out: &mut Vec<Segment>) {
        if let Some(p) = self.pending.take() {
            self.last_off = Some(*p.seg.points.last().unwrap());
            out.push(p.seg);
        }
    }

    /// Feed one programmed move (machine-coordinate polyline) through the
    /// compensator. Emits zero or more segments.
    pub fn push(&mut self, seg: Segment, out: &mut Vec<Segment>) {
        debug_assert!(self.active());
        let pts = &seg.points;
        let prog_start = pts[0];
        let prog_end = *pts.last().unwrap();

        // pure-Z (or zero-length XY) moves ride along at the offset position
        let t_overall = norm2([prog_end[0] - prog_start[0], prog_end[1] - prog_start[1]]);
        let Some(_) = t_overall else {
            self.flush(out);
            let xy = self.last_off.unwrap_or(prog_start);
            let mut seg = seg;
            seg.points = vec![[xy[0], xy[1], prog_start[2]], [xy[0], xy[1], prog_end[2]]];
            self.last_off = Some(*seg.points.last().unwrap());
            out.push(seg);
            return;
        };

        // offset every vertex along its local left normal
        let n = pts.len();
        let mut off = Vec::with_capacity(n);
        let mut tangents = Vec::with_capacity(n);
        for i in 0..n {
            let prev = if i == 0 { pts[0] } else { pts[i - 1] };
            let next = if i + 1 == n { pts[n - 1] } else { pts[i + 1] };
            let t = norm2([next[0] - prev[0], next[1] - prev[1]]).unwrap_or(t_overall.unwrap());
            tangents.push(t);
            let nl = left(t);
            off.push([
                pts[i][0] + self.side * self.radius * nl[0],
                pts[i][1] + self.side * self.radius * nl[1],
                pts[i][2],
            ]);
        }
        let t_start = tangents[0];
        let t_end = *tangents.last().unwrap();

        let mut new_seg = seg;
        new_seg.points = off;

        match self.pending.take() {
            None => {
                // entry move: from the programmed (actual) position straight
                // to the offset end of this move
                if new_seg.kind == SegmentKind::Arc {
                    new_seg.points.insert(0, prog_start);
                } else {
                    new_seg.points[0] = prog_start;
                }
            }
            Some(mut prev) => {
                let corner = prev.prog_end;
                let cross = prev.t_end[0] * t_start[1] - prev.t_end[1] * t_start[0];
                let outside = self.side * cross < -1e-9;
                let prev_end = *prev.seg.points.last().unwrap();
                let next_start = new_seg.points[0];

                if outside {
                    // join arc around the programmed corner
                    let join = self.join_arc(corner, prev_end, next_start, &new_seg);
                    self.last_off = Some(prev_end);
                    out.push(prev.seg);
                    if let Some(j) = join {
                        out.push(j);
                    }
                } else {
                    // inside corner: miter two straight offsets, else blend.
                    // anchor each line at its offset *end* — the start of an
                    // entry move sits on the programmed path, not the offset
                    let mitred = prev.seg.points.len() == 2
                        && new_seg.points.len() == 2
                        && intersect(prev_end, prev.t_end, new_seg.points[1], t_start)
                        .map(|p| {
                            let z = prev_end[2];
                            *prev.seg.points.last_mut().unwrap() = [p[0], p[1], z];
                            new_seg.points[0] = [p[0], p[1], next_start[2]];
                        })
                        .is_some();
                    if !mitred {
                        new_seg.points[0] = prev_end;
                    }
                    self.last_off = Some(*prev.seg.points.last().unwrap());
                    out.push(prev.seg);
                }
            }
        }

        self.pending = Some(Pending { seg: new_seg, prog_end, t_end });
    }

    fn join_arc(&self, corner: Point, from: Point, to: Point, template: &Segment) -> Option<Segment> {
        let a0 = (from[1] - corner[1]).atan2(from[0] - corner[0]);
        let a1 = (to[1] - corner[1]).atan2(to[0] - corner[0]);
        let mut sweep = a1 - a0;
        // G41 outside corners sweep clockwise, G42 counter-clockwise
        if self.side > 0.0 {
            while sweep > 1e-9 {
                sweep -= std::f64::consts::TAU;
            }
        } else {
            while sweep < -1e-9 {
                sweep += std::f64::consts::TAU;
            }
        }
        if sweep.abs() < 1e-6 {
            return None;
        }
        let steps = ((sweep.abs() / JOIN_STEP_RAD).ceil() as usize).max(1);
        let mut points = Vec::with_capacity(steps + 1);
        for i in 0..=steps {
            let a = a0 + sweep * i as f64 / steps as f64;
            points.push([
                corner[0] + self.radius * a.cos(),
                corner[1] + self.radius * a.sin(),
                from[2] + (to[2] - from[2]) * i as f64 / steps as f64,
            ]);
        }
        points[0] = from;
        *points.last_mut().unwrap() = to;
        let mut seg = template.clone();
        seg.kind = SegmentKind::Arc;
        seg.points = points;
        Some(seg)
    }
}

/// Intersection of two XY lines given by point + direction.
fn intersect(p1: Point, d1: [f64; 2], p2: Point, d2: [f64; 2]) -> Option<[f64; 2]> {
    let det = d1[0] * d2[1] - d1[1] * d2[0];
    if det.abs() < 1e-9 {
        return None;
    }
    let t = ((p2[0] - p1[0]) * d2[1] - (p2[1] - p1[1]) * d2[0]) / det;
    Some([p1[0] + d1[0] * t, p1[1] + d1[1] * t])
}
