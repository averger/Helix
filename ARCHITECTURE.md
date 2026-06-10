# Helix — Architecture & Design Document

## Design principles

1. **The operator's attention is the scarcest resource.** One screen, no modal
   dialogs during motion, no information that doesn't earn its pixels.
2. **State is sacred.** The machine state machine is rendered exactly as it exists
   in the controller. Buttons that can't be pressed don't pretend they can.
3. **The simulator is not a toy.** Identical interface contract as the real
   machine — same state machine, same telemetry, same G-code pipeline. If it works
   in sim, the only new variable on iron is physics.
4. **The realtime core is not ours to reinvent.** LinuxCNC's trajectory planner,
   HAL and RTAPI have two decades of spindle-hours. Helix wraps; it does not rewrite.

## Visual language

- **Canvas:** pure `#000`. Panels are `#0d0f12` with hairline `#1f242b` borders.
  No gradients on surfaces, no skeuomorphism, no chrome.
- **Accent:** a single electric cyan (`#27e0ff`) reserved for *live* things — the
  tool, the active line, the running state. Amber for warnings, red exclusively
  for e-stop/faults. Color always means something.
- **Type:** the DRO uses tabular-numeral monospace, sized to be legible at 2 m.
  Everything else is the platform sans (SF / Inter / Segoe).
- **Motion:** 150–250 ms ease-out transitions. Nothing animates while it could be
  confused with machine motion.

## Machine state machine

```
ESTOP ──reset──▶ OFF ──power──▶ ON ──home──▶ HOMED (IDLE)
  ▲                                            │  ▲
  └────────────── estop (from anywhere)        │  └─ pause/stop/done
                                       run/jog/mdi ──▶ RUNNING / JOG / MDI
```

Transitions are validated server-side; the UI only *requests* transitions.
Telemetry always carries the authoritative state.

## Protocol

### Telemetry — `WS /ws` (server → client, 30 Hz)

```json
{
  "t": 1718000000.123,
  "state": "idle",
  "position": {"x": 12.4, "y": -3.1, "z": 5.0},
  "spindle": {"rpm": 8000, "on": true, "load": 0.34},
  "feed": {"actual": 1200, "programmed": 1500, "override": 1.0},
  "rapid_override": 1.0,
  "spindle_override": 1.0,
  "program": {"name": "flange.ngc", "line": 117, "total": 1240, "progress": 0.094},
  "alarms": []
}
```

### Commands — `WS /ws` (client → server)

```json
{"cmd": "estop"} | {"cmd": "estop_reset"} | {"cmd": "power", "on": true}
{"cmd": "home"}                       // all axes
{"cmd": "jog", "axis": "x", "dir": 1, "velocity": 3000}   // continuous; dir 0 stops
{"cmd": "jog_step", "axis": "z", "dir": -1, "step": 0.1}
{"cmd": "run"} | {"cmd": "pause"} | {"cmd": "resume"} | {"cmd": "stop"}
{"cmd": "mdi", "text": "G0 X0 Y0"}
{"cmd": "override", "kind": "feed" | "rapid" | "spindle", "value": 1.25}
```

### REST

| Route                   | Purpose                                        |
|-------------------------|------------------------------------------------|
| `GET  /api/programs`    | program library                                |
| `GET  /api/programs/{name}` | G-code source                              |
| `POST /api/programs/{name}/load` | parse, plan, stage for run            |
| `POST /api/programs` (multipart) | upload a program                      |
| `GET  /api/toolpath`    | planned toolpath of the loaded program — typed
                            polyline segments (`rapid` / `feed` / `arc`) for the
                            WebGL viewer                                   |

## G-code pipeline

`server/src/gcode.rs` implements a single-pass interpreter for the moves the
viewer and simulator need: `G0 G1 G2 G3 G17/18/19 G20/21 G90/91 G90.1/91.1 M2/30 F S`
with modal state, IJK/R arcs, helical interpolation and inch/metric handling.
Arcs are tessellated server-side (chord-error-bounded, 0.05 mm) so the client
renders only polylines. The same segment list drives the simulator's motion
integration (per-segment velocity at the programmed feed, rapids at machine
limits) — the progress you see in sim is the progress you'd get on iron, minus
acceleration physics.

On a real machine the pipeline hands the file to LinuxCNC's interpreter instead
and the segment list is used for visualization only.

## The machine abstraction

`server/src/machine/mod.rs` defines the `Machine` trait — ~15 async methods
(`estop`, `power`, `home`, `jog`, `run`, `telemetry()` …). Implementations:

- `machine/sim.rs` — integrates motion at 120 Hz on a tokio task, publishes at
  30 Hz. Homing sequences, jog clamping to soft limits, feed/rapid/spindle
  overrides, pause that actually holds position.
- `machine/linuxcnc.rs` — maps the same trait onto `linuxcncrsh`, the
  remote-command TCP service that ships with the LinuxCNC core. Pure Rust, no
  Python bridge; selected with `--machine linuxcnc --connect host:5007`.

Adding a different controller later (e.g. a PlanetCNC bridge) means implementing
one trait.

## Front-end

- **Stack:** Svelte 5 + Vite, plain Svelte stores for telemetry (one store, fed
  by a single WebSocket), `three` for the viewport. No component library — the
  design system is a few hundred lines of CSS custom properties and primitives.
- **Layout:** top status bar (brand, state pill, e-stop), center WebGL viewport
  with the program library as an overlay drawer, right column DRO + jog + MDI +
  overrides, bottom transport bar. Everything visible at once.
- **Viewport:** Z-up machine convention, dark grid, planned path as typed
  polylines (rapids dashed-dim, feed in white, completed in accent), live tool
  position with glow, orbit/pan/zoom with inertial damping.
