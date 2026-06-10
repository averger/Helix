//! Simulated machine.
//!
//! Integrates motion at 120 Hz on a tokio task and honours the same state
//! machine and command contract as the LinuxCNC adapter. Geometry comes from
//! the same G-code pipeline the viewer uses, so simulated progress matches
//! the plan.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{reject, Machine, Result, State};
use crate::gcode::{Program, Segment, SegmentKind};

const TICK_HZ: f64 = 120.0;
const SOFT_MIN: [f64; 3] = [-200.0, -200.0, -120.0];
const SOFT_MAX: [f64; 3] = [200.0, 200.0, 0.0];
const RAPID_MMPM: f64 = 8000.0;
const HOME_MMPM: f64 = 3000.0;
/// Used if a feed move is programmed before any F word.
const DEFAULT_FEED_MMPM: f64 = 1000.0;
const SIM_SPINDLE_RPM: f64 = 8000.0;

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
    pos: [f64; 3],
    homed: bool,

    jog_vel: [f64; 3],               // signed mm/min per axis
    jog_steps: Vec<(usize, f64)>,    // queued (axis, signed mm)

    feed_override: f64,
    rapid_override: f64,
    spindle_override: f64,
    spindle_on: bool,
    spindle_rpm: f64,
    feed_actual: f64,
    feed_programmed: f64,

    program: Option<Program>,
    run_segments: Vec<Segment>,
    seg_index: usize,
    point_index: usize,
    dist_into_edge: f64,
    dist_done: f64,

    alarms: Vec<String>,
}

impl Sim {
    fn new() -> Self {
        Self {
            state: State::Estop,
            pos: [0.0; 3],
            homed: false,
            jog_vel: [0.0; 3],
            jog_steps: Vec::new(),
            feed_override: 1.0,
            rapid_override: 1.0,
            spindle_override: 1.0,
            spindle_on: false,
            spindle_rpm: 0.0,
            feed_actual: 0.0,
            feed_programmed: 0.0,
            program: None,
            run_segments: Vec::new(),
            seg_index: 0,
            point_index: 0,
            dist_into_edge: 0.0,
            dist_done: 0.0,
            alarms: Vec::new(),
        }
    }

    fn require(&self, states: &[State], action: &str) -> Result {
        if states.contains(&self.state) {
            Ok(())
        } else {
            Err(reject(format!("cannot {action} while {}", self.state.as_str())))
        }
    }

    fn halt_motion(&mut self) {
        self.jog_vel = [0.0; 3];
        self.jog_steps.clear();
        self.stage(Vec::new());
        self.feed_actual = 0.0;
    }

    fn stage(&mut self, segments: Vec<Segment>) {
        self.run_segments = segments.into_iter().filter(|s| s.length > 1e-9).collect();
        self.seg_index = 0;
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
        if self.spindle_on {
            let target = SIM_SPINDLE_RPM * self.spindle_override;
            self.spindle_rpm += (target - self.spindle_rpm) * (4.0 * dt).min(1.0);
        } else {
            self.spindle_rpm = (self.spindle_rpm - SIM_SPINDLE_RPM * 2.0 * dt).max(0.0);
        }
    }

    fn tick_homing(&mut self, dt: f64) {
        let step = HOME_MMPM / 60.0 * dt;
        let mut done = true;
        for p in self.pos.iter_mut() {
            if p.abs() <= step {
                *p = 0.0;
            } else {
                *p -= step.copysign(*p);
                done = false;
            }
        }
        if done {
            self.homed = true;
            self.state = State::Idle;
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

    fn tick_segments(&mut self, dt: f64) {
        if self.seg_index >= self.run_segments.len() {
            self.finish_motion();
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
        self.feed_programmed = if seg.kind == SegmentKind::Rapid { 0.0 } else if seg.feed > 0.0 { seg.feed } else { DEFAULT_FEED_MMPM };
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
        }
        self.stage(Vec::new());
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
            s.state = if s.homed { State::Idle } else { State::On };
        } else {
            if matches!(s.state, State::Running | State::Paused | State::Homing) {
                return Err(reject("cannot power off while in motion"));
            }
            if s.state != State::Estop {
                s.state = State::Off;
            }
        }
        Ok(())
    }

    async fn home(&self) -> Result {
        let mut s = self.lock();
        s.require(&[State::On, State::Idle], "home")?;
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
        let prog = crate::gcode::parse("(mdi)", text).map_err(|e| reject(e.to_string()))?;
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
        s.stage(segments);
        s.state = State::Mdi;
        Ok(())
    }

    async fn load_program(&self, program: Program) -> Result {
        let mut s = self.lock();
        if matches!(s.state, State::Running | State::Paused) {
            return Err(reject("cannot load a program while one is running"));
        }
        s.program = Some(program);
        s.stage(Vec::new());
        Ok(())
    }

    async fn run(&self) -> Result {
        let mut s = self.lock();
        s.require(&[State::Idle], "run")?;
        let segments = match &s.program {
            Some(p) if !p.segments.is_empty() => p.segments.clone(),
            _ => return Err(reject("no program loaded")),
        };
        s.stage(segments);
        s.spindle_on = true;
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
            s.state = State::Idle;
        }
        Ok(())
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
        let load = if s.spindle_on { 0.25 + 0.15 * (now * 2.1).sin() } else { 0.0 };
        json!({
            "t": now,
            "state": s.state.as_str(),
            "homed": s.homed,
            "position": {
                "x": (s.pos[0] * 1e4).round() / 1e4,
                "y": (s.pos[1] * 1e4).round() / 1e4,
                "z": (s.pos[2] * 1e4).round() / 1e4,
            },
            "spindle": {
                "on": s.spindle_on,
                "rpm": s.spindle_rpm.round(),
                "load": (load * 1e3).round() / 1e3,
            },
            "feed": {
                "actual": (s.feed_actual * 10.0).round() / 10.0,
                "programmed": (s.feed_programmed * 10.0).round() / 10.0,
                "override": s.feed_override,
            },
            "rapid_override": s.rapid_override,
            "spindle_override": s.spindle_override,
            "program": program,
            "alarms": s.alarms,
        })
    }
}
