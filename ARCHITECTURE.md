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
{"cmd": "home"} | {"cmd": "home", "axis": "z"}
{"cmd": "jog", "axis": "x", "dir": 1, "velocity": 3000}   // continuous; dir 0 stops
{"cmd": "jog_step", "axis": "z", "dir": -1, "step": 0.1}
{"cmd": "run"} | {"cmd": "run", "line": 117}              // run-from-line
{"cmd": "pause"} | {"cmd": "resume"} | {"cmd": "stop"}
{"cmd": "single_block", "on": true}   // pause after every block
{"cmd": "optional_stop", "on": true}  // honour M1
{"cmd": "block_delete", "on": true}   // skip /-lines at next (re)load
{"cmd": "mdi", "text": "G0 X0 Y0"}    // executed in work coordinates
{"cmd": "wcs", "index": 1}            // 0 = G54 … 8 = G59.3
{"cmd": "touch_off", "axis": "x", "value": 0}   // here becomes X0 in the active WCS
{"cmd": "tool", "number": 2}          // manual M6, length from the tool table
{"cmd": "spindle", "on": true, "rpm": 12000, "reverse": false}   // M3/M4/M5
{"cmd": "coolant", "on": true}        // M8/M9
{"cmd": "override", "kind": "feed" | "rapid" | "spindle", "value": 1.25}
```

Telemetry carries both `position` (work coordinates — what the operator
machines in) and `machine_position` (G53), plus the active `wcs`, current
`tool`, `coolant`, `single_block`/`optional_stop` latches and per-axis homing.

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
| `GET/PUT /api/tools`    | tool table (number, diameter, length, note) — lengths
                            feed G43 in programs and manual tool changes  |

## G-code pipeline

`server/src/gcode.rs` implements a single-pass interpreter for the moves the
viewer and simulator need: `G0–G3` (IJK/R arcs, helical), plane select, units,
abs/rel, **work coordinate systems** (`G54–G59.3`, `G10 L2/L20`, `G92/G92.1`),
**tool length compensation** (`G43 H`/`G49`, `T`/`M6`), **canned cycles**
(`G81/G82/G83` with `G98/G99` retract), program pauses (`M0/M1`), spindle and
coolant words (`M3/4/5 S`, `M7/8/9`), and block delete (`/`-lines).

Coordinates are resolved to machine space at parse time against a [`Context`]
snapshot taken from the live machine (offsets, G92, tool lengths) — exactly
what a controller's interpreter does, so the viewer shows the program where
the machine will actually cut it. Arcs are tessellated server-side
(chord-error-bounded, 0.05 mm) so the client renders only polylines. The same
segment list drives the simulator's motion integration (per-segment velocity
at the programmed feed, rapids at machine limits; spindle, coolant and tool
state per segment) — the progress you see in sim is the progress you'd get on
iron, minus acceleration physics.

On a real machine the pipeline hands the file to LinuxCNC's interpreter instead
and the segment list is used for visualization only.

## The machine abstraction

`server/src/machine/mod.rs` defines the `Machine` trait — ~20 async methods
(`estop`, `power`, `home`, `jog`, `run`, `touch_off`, `set_wcs`,
`select_tool`, `set_spindle`, `probe_z`, `telemetry()` …). Implementations:

- `machine/sim.rs` — integrates motion at 120 Hz on a tokio task, publishes at
  30 Hz. Homing sequences, jog clamping to soft limits, feed/rapid/spindle
  overrides, pause that actually holds position.
- `machine/linuxcnc.rs` — maps the same trait onto `linuxcncrsh`, the
  remote-command TCP service that ships with the LinuxCNC core. Pure Rust, no
  Python bridge; selected with `--machine linuxcnc --connect host:5007`.

Adding a different controller later (e.g. a PlanetCNC bridge) means implementing
one trait.

## Digitizing (`server/src/scan.rs`)

The operator zeroes the spindle on the part datum (0,0,0 on top of the part)
and opens a scan session. Two capture modes, freely mixed:

- **Manual trace** — jog to the surface and capture the current position,
  point by point (`POST /api/scans/active/point`), or enable auto-capture in
  the UI to record a point every *n* mm of travel while jogging along an edge.
- **Adaptive grid** — define a region, base step and probe limits; the server
  rasters it in serpentine rows through `Machine::probe_z()`. After each row it
  probes midpoints wherever two neighbours differ by more than `refine_dz`,
  repeating down to step/8 — resolution follows the shape, so flat areas stay
  sparse and edges get dense.

`probe_z()` is part of the machine contract: the simulator probes a built-in
virtual part (a dome and a flat-topped boss on a plate — see
`sim::virtual_part_height`), the LinuxCNC adapter issues `G38.3` probe moves
over linuxcncrsh and reads back the contact position.

Scans persist as JSON (`server/scans/`) and export back into the program
library as G-code (`POST /api/scans/{name}/export`): **contour** replays a
manual trace as one polyline; **raster** mills the captured rows serpentine,
Z following the digitized surface. The exported `.ngc` is immediately loadable
and runnable — digitize on the left of the shop, reproduce on the right.

Grid-scan progress streams through the telemetry channel (a `scan` field is
merged into every snapshot) and the UI renders the growing cloud live,
depth-colored, in the WebGL viewport.

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
