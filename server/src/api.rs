//! REST + WebSocket front door for the machine.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Multipart, Path as UrlPath, State as AxumState};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use crate::gcode;
use crate::machine::Machine;
use crate::scan::{self, ScanManager};

const TELEMETRY_HZ: f64 = 30.0;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Tool {
    pub number: u16,
    #[serde(default)]
    pub diameter: f64,
    #[serde(default)]
    pub length: f64,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone)]
pub struct App {
    pub machine: Arc<dyn Machine>,
    pub programs_dir: PathBuf,
    pub loaded: Arc<Mutex<Option<gcode::Program>>>,
    pub scans: Arc<ScanManager>,
    pub tools: Arc<Mutex<Vec<Tool>>>,
    pub tools_path: PathBuf,
    pub block_delete: Arc<std::sync::atomic::AtomicBool>,
}

impl App {
    pub fn load_tools(path: &std::path::Path) -> Vec<Tool> {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
}

pub fn router(app: App) -> Router {
    Router::new()
        .route("/api/programs", get(list_programs).post(upload_program))
        .route("/api/programs/:name", get(get_program))
        .route("/api/programs/:name/load", post(load_program))
        .route("/api/toolpath", get(toolpath))
        .route("/api/tools", get(get_tools).put(put_tools))
        .route("/api/scans", get(list_scans).post(create_scan))
        .route("/api/scans/active", get(get_active_scan))
        .route("/api/scans/active/point", post(capture_point))
        .route("/api/scans/active/undo", post(undo_point))
        .route("/api/scans/active/finish", post(finish_scan))
        .route("/api/scans/active/discard", post(discard_scan))
        .route("/api/scans/active/grid", post(start_grid_scan))
        .route("/api/scans/active/cancel", post(cancel_grid_scan))
        .route("/api/scans/:name", get(get_scan).delete(delete_scan))
        .route("/api/scans/:name/export", post(export_scan))
        .route("/ws", get(ws_upgrade))
        .layer(CorsLayer::permissive())
        .with_state(app)
}

type ApiError = (StatusCode, Json<Value>);

fn fail(code: StatusCode, detail: impl Into<String>) -> ApiError {
    (code, Json(json!({"detail": detail.into()})))
}

fn resolve(dir: &Path, name: &str) -> Result<PathBuf, ApiError> {
    let file = Path::new(name)
        .file_name()
        .ok_or_else(|| fail(StatusCode::BAD_REQUEST, "bad program name"))?;
    let path = dir.join(file);
    if !path.is_file() {
        return Err(fail(StatusCode::NOT_FOUND, format!("no program named {name:?}")));
    }
    Ok(path)
}

async fn list_programs(AxumState(app): AxumState<App>) -> Result<Json<Value>, ApiError> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&app.programs_dir)
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_gcode = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e, "ngc" | "nc" | "gcode"));
        if is_gcode {
            out.push(json!({
                "name": path.file_name().unwrap().to_string_lossy(),
                "size": entry.metadata().map(|m| m.len()).unwrap_or(0),
            }));
        }
    }
    out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(Json(Value::Array(out)))
}

async fn get_program(
    AxumState(app): AxumState<App>,
    UrlPath(name): UrlPath<String>,
) -> Result<Json<Value>, ApiError> {
    let path = resolve(&app.programs_dir, &name)?;
    let source = std::fs::read_to_string(path)
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({"name": name, "source": source})))
}

async fn upload_program(
    AxumState(app): AxumState<App>,
    mut multipart: Multipart,
) -> Result<Json<Value>, ApiError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| fail(StatusCode::BAD_REQUEST, e.to_string()))?
    {
        let Some(filename) = field.file_name().map(str::to_owned) else { continue };
        if !filename.ends_with(".ngc") && !filename.ends_with(".nc") && !filename.ends_with(".gcode") {
            return Err(fail(StatusCode::BAD_REQUEST, "expected a .ngc/.nc/.gcode file"));
        }
        let bytes = field.bytes().await.map_err(|e| fail(StatusCode::BAD_REQUEST, e.to_string()))?;
        let safe = Path::new(&filename).file_name().unwrap().to_string_lossy().to_string();
        std::fs::write(app.programs_dir.join(&safe), &bytes)
            .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        return Ok(Json(json!({"name": safe})));
    }
    Err(fail(StatusCode::BAD_REQUEST, "no file in upload"))
}

async fn load_program(
    AxumState(app): AxumState<App>,
    UrlPath(name): UrlPath<String>,
) -> Result<Json<Value>, ApiError> {
    let path = resolve(&app.programs_dir, &name)?;
    let source = std::fs::read_to_string(path)
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    // resolve work offsets, G92 and tool lengths exactly as the machine
    // will see them right now
    let mut ctx = app.machine.parse_context();
    for tool in app.tools.lock().unwrap().iter() {
        ctx.tool_lengths.insert(tool.number, tool.length);
        ctx.tool_diameters.insert(tool.number, tool.diameter);
    }
    ctx.block_delete = app.block_delete.load(std::sync::atomic::Ordering::Relaxed);
    let program = gcode::parse_with(&name, &source, ctx)
        .map_err(|e| fail(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    if program.segments.is_empty() {
        return Err(fail(StatusCode::UNPROCESSABLE_ENTITY, "program contains no motion"));
    }
    app.machine
        .load_program(program.clone())
        .await
        .map_err(|e| fail(StatusCode::CONFLICT, e.to_string()))?;
    let info = json!({
        "name": name,
        "lines": program.total_lines,
        "segments": program.segments.len(),
        "length_mm": (program.length() * 10.0).round() / 10.0,
        "extent": {"min": program.extent_min, "max": program.extent_max},
    });
    *app.loaded.lock().unwrap() = Some(program);
    Ok(Json(info))
}

async fn toolpath(AxumState(app): AxumState<App>) -> Json<Value> {
    let loaded = app.loaded.lock().unwrap();
    match loaded.as_ref() {
        None => Json(json!({"segments": []})),
        Some(p) => Json(json!({
            "name": p.name,
            "extent": {"min": p.extent_min, "max": p.extent_max},
            "segments": p.segments,
        })),
    }
}

// ── tool table ────────────────────────────────────────────────────────────

async fn get_tools(AxumState(app): AxumState<App>) -> Json<Vec<Tool>> {
    Json(app.tools.lock().unwrap().clone())
}

async fn put_tools(
    AxumState(app): AxumState<App>,
    Json(mut tools): Json<Vec<Tool>>,
) -> Result<Json<Value>, ApiError> {
    tools.retain(|t| t.number > 0);
    tools.sort_by_key(|t| t.number);
    tools.dedup_by_key(|t| t.number);
    std::fs::write(&app.tools_path, serde_json::to_string_pretty(&tools).unwrap())
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let count = tools.len();
    *app.tools.lock().unwrap() = tools;
    Ok(Json(json!({"tools": count})))
}

// ── digitizing ────────────────────────────────────────────────────────────

async fn list_scans(AxumState(app): AxumState<App>) -> Json<Value> {
    Json(Value::Array(app.scans.list()))
}

#[derive(serde::Deserialize)]
struct CreateScan {
    name: String,
}

async fn create_scan(
    AxumState(app): AxumState<App>,
    Json(req): Json<CreateScan>,
) -> Result<Json<Value>, ApiError> {
    let name = req.name.trim();
    if name.is_empty() || name.contains(['/', '\\']) {
        return Err(fail(StatusCode::BAD_REQUEST, "bad scan name"));
    }
    let mut active = app.scans.active.lock().unwrap();
    if active.is_some() {
        return Err(fail(StatusCode::CONFLICT, "a scan session is already open"));
    }
    *active = Some(scan::Scan {
        name: name.to_string(),
        mode: scan::ScanMode::Manual,
        created: now(),
        points: Vec::new(),
        rows: Vec::new(),
    });
    Ok(Json(json!({"name": name})))
}

async fn get_active_scan(AxumState(app): AxumState<App>) -> Json<Value> {
    let active = app.scans.active.lock().unwrap();
    match active.as_ref() {
        None => Json(json!(null)),
        Some(s) => Json(serde_json::to_value(s).unwrap()),
    }
}

async fn capture_point(AxumState(app): AxumState<App>) -> Result<Json<Value>, ApiError> {
    let snapshot = app.machine.telemetry();
    let pos = &snapshot["position"];
    let point = [
        pos["x"].as_f64().unwrap_or(0.0),
        pos["y"].as_f64().unwrap_or(0.0),
        pos["z"].as_f64().unwrap_or(0.0),
    ];
    let mut active = app.scans.active.lock().unwrap();
    let scan = active.as_mut().ok_or_else(|| fail(StatusCode::CONFLICT, "no open scan session"))?;
    // ignore a capture that didn't move
    if scan.points.last().is_some_and(|l| {
        (l[0] - point[0]).abs() < 1e-6 && (l[1] - point[1]).abs() < 1e-6 && (l[2] - point[2]).abs() < 1e-6
    }) {
        return Ok(Json(json!({"points": scan.points.len(), "added": false})));
    }
    scan.points.push(point);
    Ok(Json(json!({"points": scan.points.len(), "added": true, "point": point})))
}

async fn undo_point(AxumState(app): AxumState<App>) -> Result<Json<Value>, ApiError> {
    let mut active = app.scans.active.lock().unwrap();
    let scan = active.as_mut().ok_or_else(|| fail(StatusCode::CONFLICT, "no open scan session"))?;
    scan.points.pop();
    Ok(Json(json!({"points": scan.points.len()})))
}

async fn finish_scan(AxumState(app): AxumState<App>) -> Result<Json<Value>, ApiError> {
    if app.scans.job.active.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(fail(StatusCode::CONFLICT, "grid scan still running"));
    }
    let scan = app
        .scans
        .active
        .lock()
        .unwrap()
        .take()
        .ok_or_else(|| fail(StatusCode::CONFLICT, "no open scan session"))?;
    if scan.points.is_empty() {
        return Err(fail(StatusCode::UNPROCESSABLE_ENTITY, "scan has no points"));
    }
    app.scans
        .save(&scan)
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({"name": scan.name, "points": scan.points.len()})))
}

async fn discard_scan(AxumState(app): AxumState<App>) -> Result<Json<Value>, ApiError> {
    if app.scans.job.active.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(fail(StatusCode::CONFLICT, "grid scan still running"));
    }
    app.scans.active.lock().unwrap().take();
    Ok(Json(json!({"ok": true})))
}

async fn start_grid_scan(
    AxumState(app): AxumState<App>,
    Json(params): Json<scan::GridParams>,
) -> Result<Json<Value>, ApiError> {
    use std::sync::atomic::Ordering;
    {
        let mut active = app.scans.active.lock().unwrap();
        let scan_session = active.as_mut().ok_or_else(|| fail(StatusCode::CONFLICT, "no open scan session"))?;
        if app.scans.job.active.swap(true, Ordering::SeqCst) {
            return Err(fail(StatusCode::CONFLICT, "a grid scan is already running"));
        }
        scan_session.mode = scan::ScanMode::Grid;
    }
    let (rows, ys) = scan::plan_grid(&params);
    let job = Arc::clone(&app.scans.job);
    job.cancel.store(false, Ordering::SeqCst);
    job.done.store(0, Ordering::SeqCst);
    job.total.store(rows.iter().map(Vec::len).sum(), Ordering::SeqCst);

    let machine = Arc::clone(&app.machine);
    let scans = Arc::clone(&app.scans);
    tokio::spawn(async move {
        let result = run_grid_job(&machine, &scans, &params, rows, ys).await;
        scans.job.active.store(false, Ordering::SeqCst);
        if result.is_ok() {
            // persist a snapshot so the scan survives even before "finish"
            let active = scans.active.lock().unwrap().clone();
            if let Some(s) = active {
                let _ = scans.save(&s);
            }
        }
    });
    Ok(Json(json!({"started": true})))
}

async fn run_grid_job(
    machine: &Arc<dyn Machine>,
    scans: &Arc<ScanManager>,
    params: &scan::GridParams,
    rows: Vec<Vec<f64>>,
    ys: Vec<f64>,
) -> crate::machine::Result {
    use std::sync::atomic::Ordering;
    let feed = 300.0;
    for (xs, &y) in rows.iter().zip(&ys) {
        let mut row: Vec<(f64, f64)> = Vec::with_capacity(xs.len());
        for &x in xs {
            if scans.job.cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            let z = machine.probe_z(x, y, params.z_safe, params.z_min, feed).await?;
            row.push((x, z.unwrap_or(params.z_min)));
            scans.job.done.fetch_add(1, Ordering::Relaxed);
        }
        // adaptive refinement: keep probing midpoints until the row's
        // resolution matches its relief
        loop {
            let mids = scan::refine_candidates(&row, params.refine_dz, params.step);
            if mids.is_empty() {
                break;
            }
            scans.job.total.fetch_add(mids.len(), Ordering::Relaxed);
            let descending = xs.len() >= 2 && xs[1] < xs[0];
            for x in mids {
                if scans.job.cancel.load(Ordering::Relaxed) {
                    return Ok(());
                }
                let z = machine.probe_z(x, y, params.z_safe, params.z_min, feed).await?;
                row.push((x, z.unwrap_or(params.z_min)));
                scans.job.done.fetch_add(1, Ordering::Relaxed);
            }
            row.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            if descending {
                row.reverse();
            }
        }
        let mut active = scans.active.lock().unwrap();
        if let Some(s) = active.as_mut() {
            for &(x, z) in &row {
                s.points.push([x, y, z]);
            }
            s.rows.push(row.len());
        }
    }
    Ok(())
}

async fn cancel_grid_scan(AxumState(app): AxumState<App>) -> Json<Value> {
    app.scans.job.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    Json(json!({"ok": true}))
}

async fn get_scan(
    AxumState(app): AxumState<App>,
    UrlPath(name): UrlPath<String>,
) -> Result<Json<Value>, ApiError> {
    let scan = scan_by_name(&app, &name)?;
    Ok(Json(serde_json::to_value(scan).unwrap()))
}

async fn delete_scan(
    AxumState(app): AxumState<App>,
    UrlPath(name): UrlPath<String>,
) -> Result<Json<Value>, ApiError> {
    if app.scans.delete(&name) {
        Ok(Json(json!({"ok": true})))
    } else {
        Err(fail(StatusCode::NOT_FOUND, format!("no scan named {name:?}")))
    }
}

async fn export_scan(
    AxumState(app): AxumState<App>,
    UrlPath(name): UrlPath<String>,
    Json(params): Json<scan::ExportParams>,
) -> Result<Json<Value>, ApiError> {
    let scan = scan_by_name(&app, &name)?;
    let gcode_text = scan::export_gcode(&scan, &params)
        .map_err(|e| fail(StatusCode::UNPROCESSABLE_ENTITY, e))?;
    let filename = format!("{}-{}.ngc", scan.name, params.mode);
    std::fs::write(app.programs_dir.join(&filename), &gcode_text)
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({"program": filename, "points": scan.points.len()})))
}

/// Stored scans win; fall back to the open session so the operator can
/// export without finishing first.
fn scan_by_name(app: &App, name: &str) -> Result<scan::Scan, ApiError> {
    if let Some(s) = app.scans.get(name) {
        return Ok(s);
    }
    let active = app.scans.active.lock().unwrap();
    match active.as_ref() {
        Some(s) if s.name == name => Ok(s.clone()),
        _ => Err(fail(StatusCode::NOT_FOUND, format!("no scan named {name:?}"))),
    }
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

async fn ws_upgrade(AxumState(app): AxumState<App>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_session(app, socket))
}

async fn ws_session(app: App, mut socket: WebSocket) {
    let mut ticker =
        tokio::time::interval(std::time::Duration::from_secs_f64(1.0 / TELEMETRY_HZ));
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let mut snapshot = app.machine.telemetry();
                if let Some(obj) = snapshot.as_object_mut() {
                    obj.insert("scan".into(), app.scans.status());
                    obj.insert(
                        "block_delete".into(),
                        app.block_delete.load(std::sync::atomic::Ordering::Relaxed).into(),
                    );
                }
                if socket.send(Message::Text(snapshot.to_string())).await.is_err() {
                    return;
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(msg) = serde_json::from_str::<Value>(&text) {
                            if let Err(e) = dispatch(&app, &msg).await {
                                let reply = json!({
                                    "event": "rejected",
                                    "cmd": msg.get("cmd"),
                                    "reason": e.to_string(),
                                });
                                if socket.send(Message::Text(reply.to_string())).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => return,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => return,
                }
            }
        }
    }
}

async fn dispatch(app: &App, msg: &Value) -> crate::machine::Result {
    use crate::machine::reject;
    let m = &app.machine;
    let str_of = |key: &str| msg.get(key).and_then(Value::as_str);
    let num_of = |key: &str| msg.get(key).and_then(Value::as_f64);
    let axis = || -> crate::machine::Result<char> {
        str_of("axis")
            .and_then(|s| s.chars().next())
            .ok_or_else(|| reject("missing axis"))
    };
    let bool_of = |key: &str| msg.get(key).and_then(Value::as_bool);
    match str_of("cmd") {
        Some("estop") => m.estop().await,
        Some("estop_reset") => m.estop_reset().await,
        Some("power") => m.power(bool_of("on").unwrap_or(true)).await,
        Some("home") => m.home(str_of("axis").and_then(|s| s.chars().next())).await,
        Some("jog") => {
            let dir = num_of("dir").ok_or_else(|| reject("missing dir"))? as i8;
            m.jog(axis()?, dir, num_of("velocity").unwrap_or(3000.0)).await
        }
        Some("jog_step") => {
            let dir = num_of("dir").ok_or_else(|| reject("missing dir"))? as i8;
            let step = num_of("step").ok_or_else(|| reject("missing step"))?;
            m.jog_step(axis()?, dir, step).await
        }
        Some("mdi") => m.mdi(str_of("text").ok_or_else(|| reject("missing text"))?).await,
        Some("run") => m.run(num_of("line").map(|l| l as usize)).await,
        Some("pause") => m.pause().await,
        Some("resume") => m.resume().await,
        Some("stop") => m.stop().await,
        Some("single_block") => m.set_single_block(bool_of("on").unwrap_or(false)).await,
        Some("optional_stop") => m.set_optional_stop(bool_of("on").unwrap_or(false)).await,
        Some("block_delete") => {
            // load-time switch: takes effect on the next program (re)load
            app.block_delete
                .store(bool_of("on").unwrap_or(false), std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }
        Some("wcs") => {
            let index = num_of("index").ok_or_else(|| reject("missing index"))? as usize;
            m.set_wcs(index).await
        }
        Some("touch_off") => {
            let value = num_of("value").unwrap_or(0.0);
            m.touch_off(axis()?, value).await
        }
        Some("tool") => {
            let number = num_of("number").ok_or_else(|| reject("missing number"))? as u16;
            let length = app
                .tools
                .lock()
                .unwrap()
                .iter()
                .find(|t| t.number == number)
                .map(|t| t.length)
                .unwrap_or(0.0);
            m.select_tool(number, length).await
        }
        Some("spindle") => {
            let on = bool_of("on").unwrap_or(false);
            m.set_spindle(on, num_of("rpm").unwrap_or(0.0), bool_of("reverse").unwrap_or(false)).await
        }
        Some("coolant") => m.set_coolant(bool_of("on").unwrap_or(false)).await,
        Some("override") => {
            let kind = str_of("kind").ok_or_else(|| reject("missing kind"))?;
            let value = num_of("value").ok_or_else(|| reject("missing value"))?;
            m.set_override(kind, value).await
        }
        other => Err(reject(format!("unknown command {other:?}"))),
    }
}
