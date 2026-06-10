//! Machine backend for the real iron.
//!
//! Talks to LinuxCNC through `linuxcncrsh`, the remote-command TCP service
//! that ships with the core (`core/linuxcnc/src/emc/usr_intf/emcrsh.cc`).
//! Run it on the machine with `loadusr linuxcncrsh` (or add it to the INI),
//! then start Helix with:
//!
//!     helix-server --machine linuxcnc --connect host:5007
//!
//! Everything stays in Rust — no Python bridge. The Helix-parsed segment list
//! is used for visualization only; the program itself is executed by
//! LinuxCNC's own interpreter via `set open` / `set run`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

use super::{reject, Machine, Result, State};
use crate::gcode::Program;

const JOG_VEL_UNITS_PER_S: f64 = 1.0 / 60.0; // mm/min → units/s

struct Link {
    reader: Lines<BufReader<OwnedReadHalf>>,
    writer: OwnedWriteHalf,
}

impl Link {
    async fn exchange(&mut self, line: &str) -> Result<String> {
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .await
            .map_err(|e| reject(format!("linuxcncrsh write: {e}")))?;
        match self.reader.next_line().await {
            Ok(Some(reply)) => Ok(reply),
            Ok(None) => Err(reject("linuxcncrsh closed the connection")),
            Err(e) => Err(reject(format!("linuxcncrsh read: {e}"))),
        }
    }

    async fn set(&mut self, what: &str) -> Result {
        let reply = self.exchange(&format!("set {what}")).await?;
        if reply.contains("NAK") {
            return Err(reject(format!("controller refused: set {what}")));
        }
        Ok(())
    }

    async fn get(&mut self, what: &str) -> Result<String> {
        let reply = self.exchange(&format!("get {what}")).await?;
        Ok(reply.trim().to_string())
    }
}

pub struct LinuxCncMachine {
    link: tokio::sync::Mutex<Link>,
    loaded: Mutex<Option<ProgramInfo>>,
    cached: Arc<Mutex<Value>>,
}

struct ProgramInfo {
    name: String,
    total_lines: usize,
}

impl LinuxCncMachine {
    pub async fn connect(addr: &str) -> Result<Arc<Self>> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| reject(format!("cannot reach linuxcncrsh at {addr}: {e}")))?;
        let (read, write) = stream.into_split();
        let mut link = Link { reader: BufReader::new(read).lines(), writer: write };

        // handshake (default linuxcncrsh credentials; see emcrsh docs)
        let hello = link.exchange("hello EMC helix 1.0").await?;
        if !hello.to_lowercase().contains("hello ack") {
            return Err(reject(format!("unexpected linuxcncrsh greeting: {hello}")));
        }
        link.set("enable EMCTOO").await?;

        let machine = Arc::new(Self {
            link: tokio::sync::Mutex::new(link),
            loaded: Mutex::new(None),
            cached: Arc::new(Mutex::new(json!({"state": "off", "alarms": ["connecting"]}))),
        });

        // telemetry poller — keeps `telemetry()` sync and cheap
        let poller = Arc::clone(&machine);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(33));
            loop {
                interval.tick().await;
                if let Ok(snapshot) = poller.poll().await {
                    *poller.cached.lock().unwrap() = snapshot;
                }
            }
        });
        Ok(machine)
    }

    async fn poll(&self) -> Result<Value> {
        let mut link = self.link.lock().await;
        let estop = link.get("estop").await?;          // "ESTOP ON|OFF"
        let machine_on = link.get("machine").await?;   // "MACHINE ON|OFF"
        let pos = link.get("abs_act_pos").await?;      // "ABS_ACT_POS x y z ..."
        let status = link.get("program_status").await?; // "PROGRAM_STATUS IDLE|RUNNING|PAUSED"
        let mode = link.get("mode").await?;            // "MODE MANUAL|AUTO|MDI"
        let line = link.get("program_line").await?;    // "PROGRAM_LINE n"
        let homed = link.get("joint_homed").await?;    // "JOINT_HOMED YES NO ..."
        drop(link);

        let coords: Vec<f64> = pos
            .split_whitespace()
            .skip(1)
            .take(3)
            .filter_map(|v| v.parse().ok())
            .collect();
        let estop_on = estop.to_uppercase().contains("ON");
        let power_on = machine_on.to_uppercase().contains(" ON");
        let all_homed = homed.split_whitespace().skip(1).all(|j| j.eq_ignore_ascii_case("yes"));
        let status_u = status.to_uppercase();
        let mode_u = mode.to_uppercase();

        let state = if estop_on {
            State::Estop
        } else if !power_on {
            State::Off
        } else if status_u.contains("PAUSED") {
            State::Paused
        } else if status_u.contains("RUNNING") {
            if mode_u.contains("MDI") {
                State::Mdi
            } else {
                State::Running
            }
        } else if !all_homed {
            State::On
        } else {
            State::Idle
        };

        let current_line: usize = line.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        let program = self.loaded.lock().unwrap().as_ref().map(|p| {
            json!({
                "name": p.name,
                "line": current_line,
                "total": p.total_lines,
                "progress": if p.total_lines > 0 { current_line as f64 / p.total_lines as f64 } else { 0.0 },
                "segment": -1,
            })
        });

        Ok(json!({
            "t": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64(),
            "state": state.as_str(),
            "homed": all_homed,
            "position": {
                "x": coords.first().copied().unwrap_or(0.0),
                "y": coords.get(1).copied().unwrap_or(0.0),
                "z": coords.get(2).copied().unwrap_or(0.0),
            },
            "spindle": {"on": false, "rpm": 0, "load": 0.0},
            "feed": {"actual": 0.0, "programmed": 0.0, "override": 1.0},
            "rapid_override": 1.0,
            "spindle_override": 1.0,
            "program": program,
            "alarms": [],
        }))
    }

    fn joint(axis: char) -> Result<usize> {
        match axis {
            'x' => Ok(0),
            'y' => Ok(1),
            'z' => Ok(2),
            _ => Err(reject(format!("unknown axis {axis:?}"))),
        }
    }
}

#[async_trait]
impl Machine for LinuxCncMachine {
    async fn estop(&self) -> Result {
        self.link.lock().await.set("estop on").await
    }

    async fn estop_reset(&self) -> Result {
        self.link.lock().await.set("estop off").await
    }

    async fn power(&self, on: bool) -> Result {
        self.link.lock().await.set(if on { "machine on" } else { "machine off" }).await
    }

    async fn home(&self) -> Result {
        let mut link = self.link.lock().await;
        link.set("mode manual").await?;
        link.set("home -1").await
    }

    async fn jog(&self, axis: char, direction: i8, velocity: f64) -> Result {
        let joint = Self::joint(axis)?;
        let mut link = self.link.lock().await;
        link.set("mode manual").await?;
        if direction == 0 {
            link.set(&format!("jog_stop {joint}")).await
        } else {
            let vel = velocity.abs() * JOG_VEL_UNITS_PER_S * f64::from(direction.signum());
            link.set(&format!("jog {joint} {vel:.4}")).await
        }
    }

    async fn jog_step(&self, axis: char, direction: i8, step: f64) -> Result {
        let joint = Self::joint(axis)?;
        let mut link = self.link.lock().await;
        link.set("mode manual").await?;
        let vel = 3000.0 * JOG_VEL_UNITS_PER_S * f64::from(direction.signum());
        link.set(&format!("jog_incr {joint} {vel:.4} {:.4}", step.abs())).await
    }

    async fn mdi(&self, text: &str) -> Result {
        let mut link = self.link.lock().await;
        link.set("mode mdi").await?;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            link.set(&format!("mdi {line}")).await?;
        }
        Ok(())
    }

    async fn load_program(&self, program: Program) -> Result {
        // hand the file to LinuxCNC's own interpreter; Helix's parsed
        // segments are used for visualization only
        let path = std::env::temp_dir().join(format!("helix-{}", program.name));
        std::fs::write(&path, &program.source).map_err(|e| reject(e.to_string()))?;
        let mut link = self.link.lock().await;
        link.set("mode auto").await?;
        link.set(&format!("open {}", path.display())).await?;
        *self.loaded.lock().unwrap() =
            Some(ProgramInfo { name: program.name.clone(), total_lines: program.total_lines });
        Ok(())
    }

    async fn run(&self) -> Result {
        let mut link = self.link.lock().await;
        link.set("mode auto").await?;
        link.set("run").await
    }

    async fn pause(&self) -> Result {
        self.link.lock().await.set("pause").await
    }

    async fn resume(&self) -> Result {
        self.link.lock().await.set("resume").await
    }

    async fn stop(&self) -> Result {
        self.link.lock().await.set("abort").await
    }

    async fn set_override(&self, kind: &str, value: f64) -> Result {
        let percent = (value.clamp(0.0, 2.0) * 100.0).round() as i64;
        let cmd = match kind {
            "feed" => format!("feed_override {percent}"),
            "spindle" => format!("spindle_override {percent}"),
            "rapid" => return Err(reject("linuxcncrsh has no rapid override; set it in HAL")),
            _ => return Err(reject(format!("unknown override {kind:?}"))),
        };
        self.link.lock().await.set(&cmd).await
    }

    fn telemetry(&self) -> Value {
        self.cached.lock().unwrap().clone()
    }
}
