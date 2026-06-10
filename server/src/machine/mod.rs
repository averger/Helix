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
    async fn home(&self) -> Result;

    // -- motion ----------------------------------------------------------------
    /// Continuous jog; direction 0 stops the axis. Velocity in mm/min.
    async fn jog(&self, axis: char, direction: i8, velocity: f64) -> Result;
    async fn jog_step(&self, axis: char, direction: i8, step: f64) -> Result;
    async fn mdi(&self, text: &str) -> Result;

    // -- program -------------------------------------------------------------------
    async fn load_program(&self, program: Program) -> Result;
    async fn run(&self) -> Result;
    async fn pause(&self) -> Result;
    async fn resume(&self) -> Result;
    async fn stop(&self) -> Result;

    // -- overrides ---------------------------------------------------------------------
    /// kind: "feed" | "rapid" | "spindle"; value clamped to 0.0–2.0.
    async fn set_override(&self, kind: &str, value: f64) -> Result;

    // -- telemetry -------------------------------------------------------------------------
    /// Snapshot for the 30 Hz websocket stream. Must be cheap.
    fn telemetry(&self) -> Value;
}
