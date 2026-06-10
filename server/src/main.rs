mod api;
mod gcode;
mod machine;
mod scan;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use clap::{Parser, ValueEnum};

use machine::linuxcnc::LinuxCncMachine;
use machine::sim::SimMachine;
use machine::Machine;

#[derive(Clone, Copy, ValueEnum)]
enum Backend {
    Sim,
    Linuxcnc,
}

/// Helix machine-control server.
#[derive(Parser)]
#[command(name = "helix-server", version)]
struct Args {
    /// Controller backend.
    #[arg(long, value_enum, default_value = "sim")]
    machine: Backend,

    /// linuxcncrsh address (with --machine linuxcnc).
    #[arg(long, default_value = "127.0.0.1:5007")]
    connect: String,

    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    #[arg(long, default_value_t = 8112)]
    port: u16,

    /// Program library directory.
    #[arg(long)]
    programs: Option<PathBuf>,

    /// Digitized-scan storage directory.
    #[arg(long)]
    scans: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow_lite::Result {
    tracing_subscriber::fmt().with_target(false).init();
    let args = Args::parse();

    let machine: Arc<dyn Machine> = match args.machine {
        Backend::Sim => Arc::new(SimMachine::new()),
        Backend::Linuxcnc => LinuxCncMachine::connect(&args.connect)
            .await
            .map_err(|e| format!("linuxcnc backend: {e}"))?,
    };

    let programs_dir = args.programs.unwrap_or_else(default_programs_dir);
    std::fs::create_dir_all(&programs_dir).map_err(|e| e.to_string())?;
    let scans_dir = args.scans.unwrap_or_else(default_scans_dir);
    let scans = Arc::new(scan::ScanManager::new(scans_dir).map_err(|e| e.to_string())?);
    let tools_path = default_data_dir().join("tools.json");
    let tools = api::App::load_tools(&tools_path);

    let app = api::App {
        machine,
        programs_dir: programs_dir.clone(),
        loaded: Arc::new(Mutex::new(None)),
        scans,
        tools: Arc::new(Mutex::new(tools)),
        tools_path,
        block_delete: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let addr = format!("{}:{}", args.host, args.port);
    tracing::info!("Helix server · listening on {addr} · programs={}", programs_dir.display());

    let listener = tokio::net::TcpListener::bind(&addr).await.map_err(|e| e.to_string())?;
    axum::serve(listener, api::router(app)).await.map_err(|e| e.to_string())?;
    Ok(())
}

/// `programs/` next to the binary's crate when developing, else ~/.helix/programs.
fn default_programs_dir() -> PathBuf {
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("programs");
    if dev.is_dir() {
        return dev;
    }
    dirs_home().join(".helix").join("programs")
}

fn default_scans_dir() -> PathBuf {
    default_data_dir().join("scans")
}

fn default_data_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if manifest.is_dir() {
        return manifest;
    }
    dirs_home().join(".helix")
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// Minimal error plumbing so main() reads cleanly without an extra crate.
mod anyhow_lite {
    pub type Result = std::result::Result<(), String>;
}
