//! Simulated machine.
//!
//! Integrates motion at 120 Hz on a tokio task and honours the same state
//! machine and command contract as the LinuxCNC adapter: work coordinate
//! systems, touch-off, tool length offsets, manual spindle/coolant,
//! single-block and M0/M1 pauses, per-axis homing. Geometry comes from the
//! same G-code pipeline the viewer uses, so simulated progress matches the
//! plan.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{reject, Machine, Result, State};
use crate::gcode::{Context, PauseKind, Program, Segment, SegmentKind, WCS_NAMES};

type Dwell = (usize, f64);

const TICK_HZ: f64 = 120.0;
const SOFT_MIN: [f64; 3] = [-200.0, -200.0, -120.0];
const SOFT_MAX: [f64; 3] = [200.0, 200.0, 0.0];
const RAPID_MMPM: f64 = 8000.0;
const HOME_MMPM: f64 = 3000.0;
/// Used if a feed move is programmed before any F word.
const DEFAULT_FEED_MMPM: f64 = 1000.0;
const DEFAULT_SPINDLE_RPM: f64 = 8000.0;

/// The virtual part on the simulated table, for digitizing demos: a plate
/// 20 mm below the datum carrying a gaussian dome and a flat-topped boss.
/// Height is expressed in machine coordinates (datum = spindle zero on top
/// of the part, so the surface is at z ≤ 0).
pub fn virtual_part_height(x: f64, y: f64) -> f64 {
    let base = -20.0;
    // gaussian dome, apex ≈ -1 mm
    let dome = {
        let r2 = ((x + 15.0).powi(2) + (y + 10.0).powi(2)) / (18.0f64).powi(2);
        19.0 * (-r2).exp()
    };
    // cylindrical boss with a softened shoulder, flat top at -6 mm
    let boss = {
        let r = ((x - 25.0).powi(2) + (y - 15.0).powi(2)).sqrt();
        if r <= 12.0 {
            14.0
        } else if r <= 15.0 {
            14.0 * (1.0 - (r - 12.0) / 3.0)
        } else {
            0.0
        }
    };
    base + dome.max(boss)
}

fn axis_index(axis: char) -> Result<usize> {
    match axis {
        'x' => Ok(0),
        'y' => Ok(1),
        'z' => Ok(2),
        _ => Err(reject(format!("unknown axis {axis:?}"))),
    }
}

struct Sim {
    state: State,
    /// Machine coordinates.
    pos: [f64; 3],
    homed_axes: [bool; 3],
    homing_axes: [bool; 3],

    // offsets & tooling
    wcs: [[f64; 3]; 9],
    active_wcs: usize,
    g92: [f64; 3],
    tool_no: u16,
    tool_length: f64,

    jog_vel: [f64; 3],            // signed mm/min per axis
    jog_steps: Vec<(usize, f64)>, // queued (axis, signed mm)

    feed_override: f64,
    rapid_override: f64,
    spindle_override: f64,
    spindle_on: bool,
    spindle_reverse: bool,
    /// Commanded speed (from the program segment or manual control).
    spindle_cmd_rpm: f64,
    spindle_rpm: f64,
    coolant: bool,
    feed_actual: f64,
    feed_programmed: f64,

    // execution
    program: Option<Program>,
    run_segments: Vec<Segment>,
    staged_pauses: Vec<(usize, PauseKind)>,
    staged_dwells: Vec<Dwell>,
    dwell_remaining: f64,
    seg_index: usize,
    seg_entered: usize,
    point_index: usize,
    dist_into_edge: f64,
    dist_done: f64,
    single_block: bool,
    optional_stop: bool,

    alarms: Vec<String>,
}

impl Sim {
    fn new() -> Self {
        Self {
            state: State::Estop,
            pos: [0.0; 3],
            homed_axes: [false; 3],
            homing_axes: [false; 3],
            wcs: [[0.0; 3]; 9],
            active_wcs: 0,
            g92: [0.0; 3],
            tool_no: 0,
            tool_length: 0.0,
            jog_vel: [0.0; 3],
            jog_steps: Vec::new(),
            feed_override: 1.0,
            rapid_override: 1.0,
            spindle_override: 1.0,
            spindle_on: false,
            spindle_reverse: false,
            spindle_cmd_rpm: 0.0,
            spindle_rpm: 0.0,
            coolant: false,
            feed_actual: 0.0,
            feed_programmed: 0.0,
            program: None,
            run_segments: Vec::new(),
            staged_pauses: Vec::new(),
            staged_dwells: Vec::new(),
            dwell_remaining: 0.0,
            seg_index: 0,
            seg_entered: usize::MAX,
            point_index: 0,
            dist_into_edge: 0.0,
            dist_done: 0.0,
            single_block: false,
            optional_stop: false,
            alarms: Vec::new(),
        }
    }

    fn homed(&self) -> bool {
        self.homed_axes.iter().all(|h| *h)
    }

    fn require(&self, states: &[State], action: &str) -> Result {
        if states.contains(&self.state) {
            Ok(())
        } else {
            Err(reject(format!("cannot {action} while {}", self.state.as_str())))
        }
    }

    /// Offset between work and machine coordinates per axis.
    fn work_offset(&self, i: usize) -> f64 {
        let mut o = self.wcs[self.active_wcs][i] + self.g92[i];
        if i == 2 {
            o += self.tool_length;
        }
        o
    }

    fn halt_motion(&mut self) {
        self.jog_vel = [0.0; 3];
        self.jog_steps.clear();
        self.stage(Vec::new(), Vec::new(), Vec::new());
        self.feed_actual = 0.0;
    }

    fn stage(&mut self, segments: Vec<Segment>, pauses: Vec<(usize, PauseKind)>, dwells: Vec<Dwell>) {
        // drop zero-length segments while keeping pause/dwell indices aligned
        let mut kept = Vec::with_capacity(segments.len());
        let mut remap = Vec::with_capacity(segments.len() + 1);
        for seg in segments {
            remap.push(kept.len());
            if seg.length > 1e-9 {
                kept.push(seg);
            }
        }
        remap.push(kept.len());
        let map = |i: usize| remap.get(i).copied().unwrap_or(kept.len());
        self.staged_pauses = pauses.into_iter().map(|(i, k)| (map(i), k)).collect();
        self.staged_dwells = dwells.into_iter().map(|(i, s)| (map(i), s)).collect();
        self.dwell_remaining = 0.0;
        self.run_segments = kept;
        self.seg_index = 0;
        self.seg_entered = usize::MAX;
        self.point_index = 0;
        self.dist_into_edge = 0.0;
        self.dist_done = 0.0;
    }

    fn clamp(axis: usize, value: f64) -> f64 {
        value.clamp(SOFT_MIN[axis], SOFT_MAX[axis])
    }

    fn tick(&mut self, dt: f64) {
        match self.state {
            State::Homing => self.tick_homing(dt),
            State::Jog => self.tick_jog(dt),
            State::Running | State::Mdi => self.tick_segments(dt),
            _ => {}
        }
        let target = if self.spindle_on { self.spindle_cmd_rpm * self.spindle_override } else { 0.0 };
        if self.spindle_rpm < target {
            self.spindle_rpm = (self.spindle_rpm + (target - self.spindle_rpm) * (4.0 * dt).min(1.0)).min(target);
        } else {
            self.spindle_rpm = (self.spindle_rpm - DEFAULT_SPINDLE_RPM * 2.0 * dt).max(target);
        }
    }

    fn tick_homing(&mut self, dt: f64) {
        let step = HOME_MMPM / 60.0 * dt;
        let mut done = true;
        for i in 0..3 {
            if !self.homing_axes[i] {
                continue;
            }
            if self.pos[i].abs() <= step {
                self.pos[i] = 0.0;
                self.homed_axes[i] = true;
                self.homing_axes[i] = false;
            } else {
                self.pos[i] -= step.copysign(self.pos[i]);
                done = false;
            }
        }
        if done {
            self.state = if self.homed() { State::Idle } else { State::On };
        }
    }

    fn tick_jog(&mut self, dt: f64) {
        let mut moving = false;
        for i in 0..3 {
            let v = self.jog_vel[i];
            if v != 0.0 {
                moving = true;
                self.pos[i] = Self::clamp(i, self.pos[i] + v / 60.0 * dt);
            }
        }
        // consume queued step jogs one tick-slice at a time
        if let Some(&(axis, remaining)) = self.jog_steps.first() {
            moving = true;
            let step = remaining.abs().min(HOME_MMPM / 60.0 * dt).copysign(remaining);
            self.pos[axis] = Self::clamp(axis, self.pos[axis] + step);
            let left = remaining - step;
            if left.abs() < 1e-9 {
                self.jog_steps.remove(0);
            } else {
                self.jog_steps[0] = (axis, left);
            }
        }
        if !moving {
            self.state = State::Idle;
            self.feed_actual = 0.0;
        } else {
            self.feed_actual = self.jog_vel.iter().map(|v| v * v).sum::<f64>().sqrt();
        }
    }

    /// Returns true when execution should yield (paused or finished).
    fn enter_segment(&mut self) -> bool {
        if self.seg_entered == self.seg_index {
            return false;
        }
        // M0 / armed M1 stop before this block
        if let Some(at) = self.staged_pauses.iter().position(|(i, _)| *i == self.seg_index) {
            let (_, kind) = self.staged_pauses.remove(at);
            if kind == PauseKind::Mandatory || self.optional_stop {
                if self.state == State::Running {
                    self.state = State::Paused;
                    self.feed_actual = 0.0;
                    return true;
                }
            }
        }
        // G4 dwell before this block
        if let Some(at) = self.staged_dwells.iter().position(|(i, _)| *i == self.seg_index) {
            let (_, secs) = self.staged_dwells.remove(at);
            self.dwell_remaining += secs;
        }
        self.seg_entered = self.seg_index;
        let seg = &self.run_segments[self.seg_index];
        self.spindle_on = seg.rpm > 0.0;
        if self.spindle_on {
            self.spindle_cmd_rpm = seg.rpm;
        }
        self.coolant = seg.coolant;
        if seg.tool != 0 {
            self.tool_no = seg.tool;
        }
        false
    }

    fn tick_segments(&mut self, dt: f64) {
        if self.seg_index >= self.run_segments.len() {
            self.finish_motion();
            return;
        }
        if self.enter_segment() {
            return;
        }
        if self.dwell_remaining > 0.0 {
            self.dwell_remaining -= dt;
            self.feed_actual = 0.0;
            return;
        }
        let seg = &self.run_segments[self.seg_index];
        let rate = match seg.kind {
            SegmentKind::Rapid => RAPID_MMPM * self.rapid_override,
            _ => {
                let f = if seg.feed > 0.0 { seg.feed } else { DEFAULT_FEED_MMPM };
                f * self.feed_override
            }
        };
        self.feed_programmed = if seg.kind == SegmentKind::Rapid {
            0.0
        } else if seg.feed > 0.0 {
            seg.feed
        } else {
            DEFAULT_FEED_MMPM
        };
        self.feed_actual = rate;
        let mut budget = rate / 60.0 * dt;

        while budget > 1e-12 {
            let Some(seg) = self.run_segments.get(self.seg_index) else {
                self.finish_motion();
                return;
            };
            if self.point_index >= seg.points.len() - 1 {
                self.seg_index += 1;
                self.point_index = 0;
                self.dist_into_edge = 0.0;
                if self.seg_index >= self.run_segments.len() {
                    self.finish_motion();
                    return;
                }
                // block boundary: single-block stops here, pauses checked
                if self.single_block && self.state == State::Running {
                    self.state = State::Paused;
                    self.feed_actual = 0.0;
                    return;
                }
                if self.enter_segment() {
                    return;
                }
                continue;
            }
            let a = seg.points[self.point_index];
            let b = seg.points[self.point_index + 1];
            let edge = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
            let remain = edge - self.dist_into_edge;
            let advance = budget.min(remain);
            self.dist_into_edge += advance;
            self.dist_done += advance;
            budget -= advance;
            let t = if edge > 0.0 { self.dist_into_edge / edge } else { 1.0 };
            for i in 0..3 {
                self.pos[i] = a[i] + (b[i] - a[i]) * t;
            }
            if self.dist_into_edge >= edge - 1e-12 {
                self.point_index += 1;
                self.dist_into_edge = 0.0;
            }
        }
    }

    fn finish_motion(&mut self) {
        self.feed_actual = 0.0;
        if self.state != State::Mdi {
            self.spindle_on = false;
            self.coolant = false;
        }
        self.stage(Vec::new(), Vec::new(), Vec::new());
        self.state = State::Idle;
    }
}

pub struct SimMachine {
    sim: Arc<Mutex<Sim>>,
}

impl SimMachine {
    pub fn new() -> Self {
        let sim = Arc::new(Mutex::new(Sim::new()));
        let ticker = Arc::clone(&sim);
        tokio::spawn(async move {
            let dt = 1.0 / TICK_HZ;
            let mut interval = tokio::time::interval(std::time::Duration::from_secs_f64(dt));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                ticker.lock().unwrap().tick(dt);
            }
        });
        Self { sim }
    }

    fn lock(&self) -> MutexGuard<'_, Sim> {
        self.sim.lock().unwrap()
    }
}

#[async_trait]
impl Machine for SimMachine {
    async fn estop(&self) -> Result {
        let mut s = self.lock();
        s.state = State::Estop;
        s.halt_motion();
        s.spindle_on = false;
        s.spindle_rpm = 0.0;
        s.coolant = false;
        Ok(())
    }

    async fn estop_reset(&self) -> Result {
        let mut s = self.lock();
        if s.state == State::Estop {
            s.state = State::Off;
            s.alarms.clear();
        }
        Ok(())
    }

    async fn power(&self, on: bool) -> Result {
        let mut s = self.lock();
        if on {
            s.require(&[State::Off], "power on")?;
            s.state = if s.homed() { State::Idle } else { State::On };
        } else {
            if matches!(s.state, State::Running | State::Paused | State::Homing | State::Probing) {
                return Err(reject("cannot power off while in motion"));
            }
            if s.state != State::Estop {
                s.state = State::Off;
                s.spindle_on = false;
                s.coolant = false;
            }
        }
        Ok(())
    }

    async fn home(&self, axis: Option<char>) -> Result {
        let mut s = self.lock();
        s.require(&[State::On, State::Idle], "home")?;
        match axis {
            None => s.homing_axes = [true; 3],
            Some(a) => s.homing_axes[axis_index(a)?] = true,
        }
        s.state = State::Homing;
        Ok(())
    }

    async fn jog(&self, axis: char, direction: i8, velocity: f64) -> Result {
        let i = axis_index(axis)?;
        let mut s = self.lock();
        if direction == 0 {
            s.jog_vel[i] = 0.0;
            if s.state == State::Jog && s.jog_vel.iter().all(|v| *v == 0.0) && s.jog_steps.is_empty() {
                s.state = State::Idle;
                s.feed_actual = 0.0;
            }
            return Ok(());
        }
        s.require(&[State::Idle, State::Jog], "jog")?;
        s.jog_vel[i] = velocity.abs().min(RAPID_MMPM) * f64::from(direction.signum());
        s.state = State::Jog;
        Ok(())
    }

    async fn jog_step(&self, axis: char, direction: i8, step: f64) -> Result {
        let i = axis_index(axis)?;
        let mut s = self.lock();
        s.require(&[State::Idle, State::Jog], "jog")?;
        s.jog_steps.push((i, step.abs() * f64::from(direction.signum())));
        s.state = State::Jog;
        Ok(())
    }

    async fn mdi(&self, text: &str) -> Result {
        let ctx = self.parse_context();
        let prog = crate::gcode::parse_with("(mdi)", text, ctx).map_err(|e| reject(e.to_string()))?;
        let mut s = self.lock();
        s.require(&[State::Idle], "MDI")?;
        if prog.segments.is_empty() {
            return Ok(());
        }
        // the parser starts at the origin; rebase the first move on the
        // machine's actual position so MDI rapids don't teleport
        let mut segments = prog.segments;
        if let Some(first) = segments.first_mut() {
            first.points[0] = s.pos;
            first.length = first
                .points
                .windows(2)
                .map(|w| {
                    ((w[0][0] - w[1][0]).powi(2) + (w[0][1] - w[1][1]).powi(2) + (w[0][2] - w[1][2]).powi(2)).sqrt()
                })
                .sum();
        }
        s.stage(segments, prog.pauses, prog.dwells);
        s.state = State::Mdi;
        Ok(())
    }

    async fn load_program(&self, program: Program) -> Result {
        let mut s = self.lock();
        if matches!(s.state, State::Running | State::Paused) {
            return Err(reject("cannot load a program while one is running"));
        }
        s.program = Some(program);
        s.stage(Vec::new(), Vec::new(), Vec::new());
        Ok(())
    }

    async fn run(&self, from_line: Option<usize>) -> Result {
        let mut s = self.lock();
        s.require(&[State::Idle], "run")?;
        let program = match &s.program {
            Some(p) if !p.segments.is_empty() => p.clone(),
            _ => return Err(reject("no program loaded")),
        };
        let (segments, pauses, dwells) = match from_line {
            None => (program.segments, program.pauses, program.dwells),
            Some(line) => {
                let skipped = program.segments.iter().take_while(|seg| seg.line < line).count();
                if skipped == program.segments.len() {
                    return Err(reject(format!("no motion at or after line {line}")));
                }
                let segments: Vec<_> = program.segments[skipped..].to_vec();
                let pauses = program
                    .pauses
                    .into_iter()
                    .filter(|(i, _)| *i >= skipped)
                    .map(|(i, k)| (i - skipped, k))
                    .collect();
                let dwells = program
                    .dwells
                    .into_iter()
                    .filter(|(i, _)| *i >= skipped)
                    .map(|(i, d)| (i - skipped, d))
                    .collect();
                (segments, pauses, dwells)
            }
        };
        s.stage(segments, pauses, dwells);
        s.state = State::Running;
        Ok(())
    }

    async fn pause(&self) -> Result {
        let mut s = self.lock();
        s.require(&[State::Running], "pause")?;
        s.state = State::Paused;
        s.feed_actual = 0.0;
        Ok(())
    }

    async fn resume(&self) -> Result {
        let mut s = self.lock();
        s.require(&[State::Paused], "resume")?;
        s.state = State::Running;
        Ok(())
    }

    async fn stop(&self) -> Result {
        let mut s = self.lock();
        if matches!(s.state, State::Running | State::Paused | State::Mdi | State::Jog) {
            s.halt_motion();
            s.spindle_on = false;
            s.spindle_rpm = 0.0;
            s.coolant = false;
            s.state = State::Idle;
        }
        Ok(())
    }

    async fn set_single_block(&self, on: bool) -> Result {
        self.lock().single_block = on;
        Ok(())
    }

    async fn set_optional_stop(&self, on: bool) -> Result {
        self.lock().optional_stop = on;
        Ok(())
    }

    async fn set_wcs(&self, index: usize) -> Result {
        if index >= 9 {
            return Err(reject("WCS index out of range (0–8)"));
        }
        let mut s = self.lock();
        s.require(&[State::Idle, State::Jog, State::On], "switch WCS")?;
        s.active_wcs = index;
        Ok(())
    }

    async fn touch_off(&self, axis: char, value: f64) -> Result {
        let i = axis_index(axis)?;
        let mut s = self.lock();
        s.require(&[State::Idle, State::Jog], "touch off")?;
        let tool = if i == 2 { s.tool_length } else { 0.0 };
        let wcs = s.active_wcs;
        s.wcs[wcs][i] = s.pos[i] - s.g92[i] - tool - value;
        Ok(())
    }

    async fn select_tool(&self, number: u16, length: f64) -> Result {
        let mut s = self.lock();
        s.require(&[State::Idle, State::Paused], "change tool")?;
        s.tool_no = number;
        s.tool_length = length;
        Ok(())
    }

    async fn set_spindle(&self, on: bool, rpm: f64, reverse: bool) -> Result {
        let mut s = self.lock();
        s.require(&[State::Idle, State::Paused, State::Jog], "control the spindle")?;
        s.spindle_on = on;
        s.spindle_reverse = reverse;
        if on {
            s.spindle_cmd_rpm = if rpm > 0.0 { rpm } else { DEFAULT_SPINDLE_RPM };
        }
        Ok(())
    }

    async fn set_coolant(&self, on: bool) -> Result {
        let mut s = self.lock();
        if matches!(s.state, State::Estop | State::Off) {
            return Err(reject("machine is not powered"));
        }
        s.coolant = on;
        Ok(())
    }

    async fn probe_z(&self, x: f64, y: f64, z_safe: f64, z_min: f64, _feed: f64) -> Result<Option<f64>> {
        let (x, y) = (x.clamp(SOFT_MIN[0], SOFT_MAX[0]), y.clamp(SOFT_MIN[1], SOFT_MAX[1]));
        {
            let mut s = self.lock();
            if !s.homed() {
                return Err(reject("machine must be homed to probe"));
            }
            s.require(&[State::Idle, State::Probing], "probe")?;
            s.state = State::Probing;
        }

        // The sim probes in time-lapse — a few visible hops per point so a
        // grid scan stays watchable without taking real-machine minutes.
        let pace = std::time::Duration::from_millis(15);
        let check = |s: &Sim| -> Result {
            if s.state == State::Probing {
                Ok(())
            } else {
                Err(reject("probe interrupted"))
            }
        };

        tokio::time::sleep(pace).await;
        {
            let mut s = self.lock();
            check(&s)?;
            s.pos[2] = z_safe.clamp(SOFT_MIN[2], SOFT_MAX[2]);
        }
        tokio::time::sleep(pace).await;
        {
            let mut s = self.lock();
            check(&s)?;
            s.pos[0] = x;
            s.pos[1] = y;
        }
        tokio::time::sleep(pace).await;

        let surface = virtual_part_height(x, y);
        let contact = surface >= z_min && surface <= z_safe;
        let z_stop = if contact { surface } else { z_min.max(SOFT_MIN[2]) };
        let result = {
            let mut s = self.lock();
            check(&s)?;
            s.pos[2] = z_stop;
            s.state = State::Idle;
            if contact { Some(surface) } else { None }
        };
        Ok(result)
    }

    async fn set_override(&self, kind: &str, value: f64) -> Result {
        let value = value.clamp(0.0, 2.0);
        let mut s = self.lock();
        match kind {
            "feed" => s.feed_override = value,
            "rapid" => s.rapid_override = value,
            "spindle" => s.spindle_override = value,
            _ => return Err(reject(format!("unknown override {kind:?}"))),
        }
        Ok(())
    }

    fn telemetry(&self) -> Value {
        let s = self.lock();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64();
        let executing = matches!(s.state, State::Running | State::Paused) && !s.run_segments.is_empty();
        let program = s.program.as_ref().map(|p| {
            let line = if executing && s.seg_index < s.run_segments.len() {
                s.run_segments[s.seg_index].line
            } else {
                0
            };
            let total_len = p.length().max(1.0);
            json!({
                "name": p.name,
                "line": line,
                "total": p.total_lines,
                "progress": if executing { (s.dist_done / total_len).min(1.0) } else { 0.0 },
                "segment": if executing { s.seg_index as i64 } else { -1 },
            })
        });
        let work: Vec<f64> = (0..3).map(|i| s.pos[i] - s.work_offset(i)).collect();
        let load = if s.spindle_on { 0.25 + 0.15 * (now * 2.1).sin() } else { 0.0 };
        json!({
            "t": now,
            "state": s.state.as_str(),
            "homed": s.homed(),
            "homed_axes": {"x": s.homed_axes[0], "y": s.homed_axes[1], "z": s.homed_axes[2]},
            "position": {
                "x": (work[0] * 1e4).round() / 1e4,
                "y": (work[1] * 1e4).round() / 1e4,
                "z": (work[2] * 1e4).round() / 1e4,
            },
            "machine_position": {
                "x": (s.pos[0] * 1e4).round() / 1e4,
                "y": (s.pos[1] * 1e4).round() / 1e4,
                "z": (s.pos[2] * 1e4).round() / 1e4,
            },
            "wcs": {"index": s.active_wcs, "name": WCS_NAMES[s.active_wcs]},
            "tool": {"number": s.tool_no, "length": s.tool_length},
            "spindle": {
                "on": s.spindle_on,
                "reverse": s.spindle_reverse,
                "rpm": s.spindle_rpm.round(),
                "load": (load * 1e3).round() / 1e3,
            },
            "coolant": s.coolant,
            "feed": {
                "actual": (s.feed_actual * 10.0).round() / 10.0,
                "programmed": (s.feed_programmed * 10.0).round() / 10.0,
                "override": s.feed_override,
            },
            "rapid_override": s.rapid_override,
            "spindle_override": s.spindle_override,
            "single_block": s.single_block,
            "optional_stop": s.optional_stop,
            "program": program,
            "alarms": s.alarms,
        })
    }

    fn parse_context(&self) -> Context {
        let s = self.lock();
        let mut ctx = Context {
            wcs: s.wcs,
            active_wcs: s.active_wcs,
            g92: s.g92,
            current_tool: s.tool_no,
            ..Context::default()
        };
        if s.tool_no != 0 {
            ctx.tool_lengths.insert(s.tool_no, s.tool_length);
        }
        ctx
    }
}
