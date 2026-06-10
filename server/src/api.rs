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

const TELEMETRY_HZ: f64 = 30.0;

#[derive(Clone)]
pub struct App {
    pub machine: Arc<dyn Machine>,
    pub programs_dir: PathBuf,
    pub loaded: Arc<Mutex<Option<gcode::Program>>>,
}

pub fn router(app: App) -> Router {
    Router::new()
        .route("/api/programs", get(list_programs).post(upload_program))
        .route("/api/programs/:name", get(get_program))
        .route("/api/programs/:name/load", post(load_program))
        .route("/api/toolpath", get(toolpath))
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
    let program = gcode::parse(&name, &source)
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

async fn ws_upgrade(AxumState(app): AxumState<App>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_session(app, socket))
}

async fn ws_session(app: App, mut socket: WebSocket) {
    let mut ticker =
        tokio::time::interval(std::time::Duration::from_secs_f64(1.0 / TELEMETRY_HZ));
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let snapshot = app.machine.telemetry();
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
    match str_of("cmd") {
        Some("estop") => m.estop().await,
        Some("estop_reset") => m.estop_reset().await,
        Some("power") => m.power(msg.get("on").and_then(Value::as_bool).unwrap_or(true)).await,
        Some("home") => m.home().await,
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
        Some("run") => m.run().await,
        Some("pause") => m.pause().await,
        Some("resume") => m.resume().await,
        Some("stop") => m.stop().await,
        Some("override") => {
            let kind = str_of("kind").ok_or_else(|| reject("missing kind"))?;
            let value = num_of("value").ok_or_else(|| reject("missing value"))?;
            m.set_override(kind, value).await
        }
        other => Err(reject(format!("unknown command {other:?}"))),
    }
}
