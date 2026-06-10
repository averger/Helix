# Helix

**Machine control, reimagined.**

Helix is a ground-up redesign of the CNC machine control experience, built on top of
the battle-tested [LinuxCNC](https://github.com/LinuxCNC/linuxcnc) real-time motion
engine. The realtime core that has driven mills, lathes and routers for two decades
stays. Everything the operator sees and touches is new.

![status](https://img.shields.io/badge/status-alpha-blueviolet)
![license](https://img.shields.io/badge/license-GPLv2-blue)

---

## Why

CNC control software looks like it was frozen in 1998 — grey dialog boxes, nested
menus, screens designed for a mouse on a machine you operate with greasy fingers.

Helix asks one question: *what would this look like if it were designed today?*

- **One screen.** Everything the operator needs — position, program, toolpath,
  overrides — visible at once. No menu diving while the spindle is running.
- **Touch-first.** Large targets, gestures for the 3D viewport, no right-click anywhere.
- **Dark by design.** Built for the shop floor: pure black background, high-contrast
  readouts you can read from two meters away, no glare on the enclosure glass.
- **Honest state.** The machine's state machine (ESTOP → OFF → ON → HOMED → RUNNING)
  is always visible and always truthful. No ambiguous buttons.
- **Real shop workflow.** Work coordinate systems (G54–G59.3) with one-touch
  touch-off, a tool table feeding G43 and manual tool changes, manual
  spindle/coolant, single-block, run-from-line, M0/M1 stops, block delete,
  canned drilling cycles — the things you actually need to make a part.
- **Simulation built in.** The full interface runs against a physics-faithful G-code
  simulator — train operators, verify programs, demo the UI, all without a machine.
- **Digitize what's on the table.** Zero the spindle on a part, trace it point by
  point or let the probe raster it on an adaptive grid (dense where the shape
  changes, sparse where it doesn't), then export the captured cloud straight back
  into the program library as G-code and reproduce the part.

## Architecture

```
┌─────────────────────────────────────────────────┐
│  app/        Helix UI — Svelte 5 + Three.js     │
│              WebGL toolpath, DRO, jog, program  │
└──────────────────────┬──────────────────────────┘
                       │ WebSocket (30 Hz telemetry) + REST
┌──────────────────────┴──────────────────────────┐
│  server/     Helix Server — Rust / axum / tokio │
│              state machine, G-code pipeline      │
│   ┌──────────────┐      ┌─────────────────────┐ │
│   │ sim machine  │  or  │ linuxcnc adapter    │ │
│   └──────────────┘      └──────────┬──────────┘ │
└─────────────────────────────────────┼────────────┘
                                      │ linuxcncrsh (TCP)
┌─────────────────────────────────────┴────────────┐
│  core/linuxcnc   Real-time motion engine (fork)  │
│                  trajectory planner, HAL, RTAPI  │
└──────────────────────────────────────────────────┘
```

The UI never talks to hardware directly. The Helix server exposes a single,
clean machine abstraction (the `Machine` trait in `server/src/machine/mod.rs`);
the same contract is implemented by the built-in simulator and by the LinuxCNC
adapter, so the front-end is identical in simulation and on iron.

## Quick start (simulation — no machine, no realtime kernel needed)

```bash
# 1. server (Rust ≥ 1.75)
cd server
cargo run                          # ws + REST on :8112

# 2. interface (Node ≥ 20)
cd ../app
npm install
npm run dev                        # UI on :5173, proxied to the server
```

Open http://localhost:5173, press **⏻** to bring the (simulated) machine out of
e-stop, home it, load a program from the library, press **Run**.

## Running on a real machine

On a machine with a working LinuxCNC installation (`core/linuxcnc`, see its
docs for the realtime setup), load the remote-command service that ships with
the core — `loadusr linuxcncrsh` in HAL, or add it to your INI — then:

```bash
helix-server --machine linuxcnc --connect 127.0.0.1:5007
```

The adapter speaks linuxcncrsh's TCP protocol — pure Rust, no Python bridge —
and the entire Helix interface drives the real iron. See
[ARCHITECTURE.md](ARCHITECTURE.md) for details.

## Repository layout

| Path             | What it is                                                   |
|------------------|--------------------------------------------------------------|
| `app/`           | Helix interface (Svelte 5, Vite, Three.js)                   |
| `server/`        | Helix server (Rust: axum + tokio, WS telemetry, G-code)      |
| `core/linuxcnc`  | LinuxCNC fork — real-time motion core (git submodule)        |
| `ARCHITECTURE.md`| Design document: principles, state machine, protocol         |

## License

GPLv2, inherited from LinuxCNC. The Helix server and interface are © the Helix
contributors and released under the same license.
