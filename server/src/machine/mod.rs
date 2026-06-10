//! The machine abstraction every controller backend implements.
//!
//! The Helix UI is written against exactly this contract; the simulator and
//! the LinuxCNC adapter are interchangeable behind it.

pub mod linuxcnc;
pub mod sim;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use crate::gcode::Program;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Estop,
    /// E-stop reset, drives unpowered.
    Off,
    /// Powered, not homed.
    On,
    Homing,
    /// Homed, ready.
    Idle,
    Jog,
    Mdi,
    Running,
    Paused,
    Probing,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Estop => "estop",
            State::Off => "off",
            State::On => "on",
            State::Homing => "homing",
            State::Idle => "idle",
            State::Jog => "jog",
            State::Mdi => "mdi",
            State::Running => "running",
            State::Paused => "paused",
            State::Probing => "probing",
        }
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
pub struct MachineError(pub String);

pub fn reject(msg: impl Into<String>) -> MachineError {
    MachineError(msg.into())
}

pub type Result<T = ()> = std::result::Result<T, MachineError>;

/// Async machine interface. All methods return Err on rejection
/// (wrong state, limits, fault) — never panic.
#[async_trait]
pub trait Machine: Send + Sync {
    // -- state transitions ---------------------------------------------------
    async fn estop(&self) -> Result;
    async fn estop_reset(&self) -> Result;
    async fn power(&self, on: bool) -> Result;
    /// Home one axis, or all of them when `axis` is None.
    async fn home(&self, axis: Option<char>) -> Result;

    // -- motion ----------------------------------------------------------------
    /// Continuous jog; direction 0 stops the axis. Velocity in mm/min.
    async fn jog(&self, axis: char, direction: i8, velocity: f64) -> Result;
    async fn jog_step(&self, axis: char, direction: i8, step: f64) -> Result;
    async fn mdi(&self, text: &str) -> Result;

    // -- program -------------------------------------------------------------------
    async fn load_program(&self, program: Program) -> Result;
    /// Start the loaded program; `from_line` skips everything before that
    /// 1-based source line.
    async fn run(&self, from_line: Option<usize>) -> Result;
    async fn pause(&self) -> Result;
    async fn resume(&self) -> Result;
    async fn stop(&self) -> Result;
    /// Pause after every block when on.
    async fn set_single_block(&self, on: bool) -> Result;
    /// Honour M1 optional stops when on.
    async fn set_optional_stop(&self, on: bool) -> Result;

    // -- offsets & tooling ---------------------------------------------------------
    /// Select the active work coordinate system (0 = G54 … 8 = G59.3).
    async fn set_wcs(&self, index: usize) -> Result;
    /// Make the current position read `value` on `axis` in the active WCS
    /// (the classic touch-off).
    async fn touch_off(&self, axis: char, value: f64) -> Result;
    /// Manual tool change: tool number and its length offset.
    async fn select_tool(&self, number: u16, length: f64) -> Result;

    // -- spindle & coolant -----------------------------------------------------------
    /// Manual spindle control; `reverse` = M4 direction.
    async fn set_spindle(&self, on: bool, rpm: f64, reverse: bool) -> Result;
    async fn set_coolant(&self, on: bool) -> Result;

    // -- probing -------------------------------------------------------------------
    /// Move to (x, y) at z_safe, then probe down toward z_min at `feed`.
    /// Returns Some(z) at the contact height, or None if the probe reached
    /// z_min without touching anything. Requires an idle, homed machine.
    async fn probe_z(&self, x: f64, y: f64, z_safe: f64, z_min: f64, feed: f64) -> Result<Option<f64>>;

    // -- overrides ---------------------------------------------------------------------
    /// kind: "feed" | "rapid" | "spindle"; value clamped to 0.0–2.0.
    async fn set_override(&self, kind: &str, value: f64) -> Result;

    // -- telemetry -------------------------------------------------------------------------
    /// Snapshot for the 30 Hz websocket stream. Must be cheap.
    fn telemetry(&self) -> Value;

    /// Offsets snapshot used to resolve program coordinates at load time.
    /// Backends that cannot report offsets fall back to identity.
    fn parse_context(&self) -> crate::gcode::Context {
        crate::gcode::Context::default()
    }
}
