//! Digitizing — physical point capture and shape reproduction.
//!
//! The operator zeroes the spindle on the part datum, then either jogs to
//! each point and captures it (manual trace) or lets Helix raster the part
//! with the probe on an adaptive grid: rows are probed at the base step and
//! refined wherever the surface changes faster than `refine_dz`, so flat
//! regions stay sparse and edges get dense — resolution follows the shape.
//!
//! A finished scan is saved as JSON and can be exported back into the
//! program library as G-code (contour or raster) to reproduce the part.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub type Point = [f64; 3];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScanMode {
    /// Operator-driven: jog to the surface, capture point by point.
    Manual,
    /// Probe-driven raster over a region with adaptive refinement.
    Grid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scan {
    pub name: String,
    pub mode: ScanMode,
    pub created: f64,
    pub points: Vec<Point>,
    /// Row lengths for grid scans, in probe order — preserves the raster
    /// structure (with adaptive points) for export.
    #[serde(default)]
    pub rows: Vec<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GridParams {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    /// Base step in mm for both axes.
    pub step: f64,
    /// Safe travel height.
    pub z_safe: f64,
    /// Deepest the probe may go without contact.
    pub z_min: f64,
    /// Refine between two probes when |Δz| exceeds this (mm). 0 disables.
    #[serde(default = "default_refine")]
    pub refine_dz: f64,
}

fn default_refine() -> f64 {
    0.8
}

/// Smallest x-spacing adaptive refinement will subdivide down to,
/// as a fraction of the base step.
const REFINE_MIN_FRACTION: f64 = 0.125;

#[derive(Debug, Clone, Deserialize)]
pub struct ExportParams {
    /// "contour" replays the trace as one polyline; "raster" mills
    /// serpentine rows (grid scans only).
    pub mode: String,
    pub feed: f64,
    pub z_safe: f64,
}

/// Progress of a running grid scan, shared with the telemetry stream.
#[derive(Default)]
pub struct JobProgress {
    pub active: AtomicBool,
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    pub cancel: AtomicBool,
}

pub struct ScanManager {
    dir: PathBuf,
    pub active: std::sync::Mutex<Option<Scan>>,
    pub job: Arc<JobProgress>,
}

impl ScanManager {
    pub fn new(dir: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir, active: std::sync::Mutex::new(None), job: Arc::new(JobProgress::default()) })
    }

    pub fn list(&self) -> Vec<Value> {
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                if entry.path().extension().is_some_and(|e| e == "json") {
                    if let Some(scan) = self.read(&entry.path()) {
                        out.push(json!({
                            "name": scan.name,
                            "mode": scan.mode,
                            "created": scan.created,
                            "points": scan.points.len(),
                        }));
                    }
                }
            }
        }
        out.sort_by(|a, b| b["created"].as_f64().partial_cmp(&a["created"].as_f64()).unwrap_or(std::cmp::Ordering::Equal));
        out
    }

    pub fn path(&self, name: &str) -> Option<PathBuf> {
        let file = Path::new(name).file_name()?;
        Some(self.dir.join(format!("{}.json", file.to_string_lossy())))
    }

    fn read(&self, path: &Path) -> Option<Scan> {
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }

    pub fn get(&self, name: &str) -> Option<Scan> {
        self.read(&self.path(name)?)
    }

    pub fn save(&self, scan: &Scan) -> std::io::Result<()> {
        let path = self.path(&scan.name).ok_or(std::io::ErrorKind::InvalidInput)?;
        std::fs::write(path, serde_json::to_string(scan).unwrap())
    }

    pub fn delete(&self, name: &str) -> bool {
        self.path(name).map(|p| std::fs::remove_file(p).is_ok()).unwrap_or(false)
    }

    pub fn status(&self) -> Value {
        let active = self.active.lock().unwrap();
        json!({
            "session": active.as_ref().map(|s| json!({
                "name": s.name,
                "mode": s.mode,
                "points": s.points.len(),
            })),
            "job": {
                "active": self.job.active.load(Ordering::Relaxed),
                "done": self.job.done.load(Ordering::Relaxed),
                "total": self.job.total.load(Ordering::Relaxed),
            },
        })
    }
}

/// Plan the base grid: serpentine rows at the base step.
/// Returns (rows of x positions, y per row).
pub fn plan_grid(p: &GridParams) -> (Vec<Vec<f64>>, Vec<f64>) {
    let (x0, x1) = (p.x0.min(p.x1), p.x0.max(p.x1));
    let (y0, y1) = (p.y0.min(p.y1), p.y0.max(p.y1));
    let step = p.step.max(0.05);
    let nx = ((x1 - x0) / step).round() as usize + 1;
    let ny = ((y1 - y0) / step).round() as usize + 1;
    let mut rows = Vec::with_capacity(ny);
    let mut ys = Vec::with_capacity(ny);
    for j in 0..ny {
        let y = y0 + (j as f64 * step).min(y1 - y0);
        let mut xs: Vec<f64> = (0..nx).map(|i| x0 + (i as f64 * step).min(x1 - x0)).collect();
        if j % 2 == 1 {
            xs.reverse(); // serpentine: halve travel between rows
        }
        rows.push(xs);
        ys.push(y);
    }
    (rows, ys)
}

/// Decide which midpoints to refine within a probed row: wherever two
/// neighbours differ by more than `refine_dz` and are still farther apart
/// than the refinement floor. Returns the x positions to probe next.
pub fn refine_candidates(row: &[(f64, f64)], refine_dz: f64, base_step: f64) -> Vec<f64> {
    if refine_dz <= 0.0 {
        return Vec::new();
    }
    let min_dx = base_step * REFINE_MIN_FRACTION;
    let mut out = Vec::new();
    for pair in row.windows(2) {
        let (x_a, z_a) = pair[0];
        let (x_b, z_b) = pair[1];
        if (z_b - z_a).abs() > refine_dz && (x_b - x_a).abs() > min_dx * 2.0 {
            out.push((x_a + x_b) / 2.0);
        }
    }
    out
}

/// Generate reproduction G-code from a scan.
pub fn export_gcode(scan: &Scan, p: &ExportParams) -> Result<String, String> {
    if scan.points.is_empty() {
        return Err("scan has no points".into());
    }
    let mut g = String::new();
    g.push_str(&format!("(Helix digitize - {} - {} points)\n", scan.name, scan.points.len()));
    g.push_str("G21 G90 G17\n");
    g.push_str(&format!("G0 Z{:.3}\n", p.z_safe));

    match p.mode.as_str() {
        "contour" => {
            let pts = dedupe(&scan.points);
            g.push_str(&format!("G0 X{:.3} Y{:.3}\n", pts[0][0], pts[0][1]));
            g.push_str(&format!("G1 Z{:.3} F{:.0}\n", pts[0][2], p.feed));
            for pt in &pts[1..] {
                g.push_str(&format!("G1 X{:.3} Y{:.3} Z{:.3}\n", pt[0], pt[1], pt[2]));
            }
        }
        "raster" => {
            if scan.mode != ScanMode::Grid || scan.rows.is_empty() {
                return Err("raster export needs a grid scan".into());
            }
            let mut idx = 0;
            for &len in &scan.rows {
                let row = &scan.points[idx..idx + len];
                idx += len;
                if row.is_empty() {
                    continue;
                }
                g.push_str(&format!("G0 Z{:.3}\n", p.z_safe));
                g.push_str(&format!("G0 X{:.3} Y{:.3}\n", row[0][0], row[0][1]));
                g.push_str(&format!("G1 Z{:.3} F{:.0}\n", row[0][2], p.feed));
                for pt in &row[1..] {
                    g.push_str(&format!("G1 X{:.3} Z{:.3}\n", pt[0], pt[2]));
                }
            }
        }
        other => return Err(format!("unknown export mode {other:?}")),
    }

    g.push_str(&format!("G0 Z{:.3}\n", p.z_safe));
    g.push_str("M2\n");
    Ok(g)
}

fn dedupe(points: &[Point]) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    for &p in points {
        if out.last().map_or(true, |l| {
            (l[0] - p[0]).abs() > 1e-6 || (l[1] - p[1]).abs() > 1e-6 || (l[2] - p[2]).abs() > 1e-6
        }) {
            out.push(p);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(step: f64) -> GridParams {
        GridParams { x0: 0.0, y0: 0.0, x1: 10.0, y1: 10.0, step, z_safe: 5.0, z_min: -20.0, refine_dz: 0.8 }
    }

    #[test]
    fn grid_plan_is_serpentine() {
        let (rows, ys) = plan_grid(&grid(5.0));
        assert_eq!(ys, vec![0.0, 5.0, 10.0]);
        assert_eq!(rows[0], vec![0.0, 5.0, 10.0]);
        assert_eq!(rows[1], vec![10.0, 5.0, 0.0]);
    }

    #[test]
    fn refinement_targets_edges_only() {
        // flat → step edge → flat: only the edge pair gets a midpoint
        let row = vec![(0.0, -1.0), (5.0, -1.1), (10.0, -8.0), (15.0, -8.05)];
        let mids = refine_candidates(&row, 0.8, 5.0);
        assert_eq!(mids, vec![7.5]);
    }

    #[test]
    fn refinement_respects_floor() {
        let row = vec![(0.0, 0.0), (1.0, -5.0)];
        // base step 5 → floor 0.625 → spacing 1.0 < 2*0.625 → no refinement
        assert!(refine_candidates(&row, 0.8, 5.0).is_empty());
    }

    #[test]
    fn contour_export_replays_trace() {
        let scan = Scan {
            name: "edge".into(),
            mode: ScanMode::Manual,
            created: 0.0,
            points: vec![[0.0, 0.0, -1.0], [0.0, 0.0, -1.0], [10.0, 0.0, -1.0]],
            rows: vec![],
        };
        let g = export_gcode(&scan, &ExportParams { mode: "contour".into(), feed: 600.0, z_safe: 5.0 }).unwrap();
        assert!(g.contains("G1 X10.000 Y0.000 Z-1.000"));
        // duplicate point removed: exactly one G1 move between points
        assert_eq!(g.matches("G1 X").count(), 1);
    }

    #[test]
    fn raster_export_follows_rows() {
        let scan = Scan {
            name: "boss".into(),
            mode: ScanMode::Grid,
            created: 0.0,
            points: vec![[0.0, 0.0, -2.0], [5.0, 0.0, -2.5], [5.0, 5.0, -3.0], [0.0, 5.0, -3.5]],
            rows: vec![2, 2],
        };
        let g = export_gcode(&scan, &ExportParams { mode: "raster".into(), feed: 800.0, z_safe: 5.0 }).unwrap();
        assert!(g.contains("G1 X5.000 Z-2.500"));
        assert!(g.contains("G0 X5.000 Y5.000"));
    }

    #[test]
    fn raster_rejects_manual_scans() {
        let scan = Scan { name: "t".into(), mode: ScanMode::Manual, created: 0.0, points: vec![[0.0; 3]], rows: vec![] };
        assert!(export_gcode(&scan, &ExportParams { mode: "raster".into(), feed: 800.0, z_safe: 5.0 }).is_err());
    }
}
