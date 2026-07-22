//! Supervised tunnel worker.
//!
//! Runs on a background thread. Owns the b2c2 SSM tunnels and the health
//! checks. Never panics the process: every failure becomes a `Status` the UI
//! can display. When the AWS SSO session expires the worker detects it, asks
//! for re-authentication (which can pop a browser because we run in the GUI
//! session), notifies the user, and rebuilds the tunnels with backoff.

use std::{
    fs::{File, OpenOptions},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc::{Receiver, RecvTimeoutError},
    time::{Duration, Instant},
};

use derive_new::new;
use wait_timeout::ChildExt;

/// How long write (primary) mode stays active before auto-reverting to
/// read-only, as a safety guard against leaving write access on.
const WRITE_TTL: Duration = Duration::from_secs(30 * 60);

/// High-level state of the worker, surfaced to the menu-bar UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Not running; waiting for a Start command.
    Stopped,
    /// Bringing tunnels up.
    Starting,
    /// Waiting on interactive AWS SSO login (browser may be open).
    LoggingIn,
    /// All tunnels up and health checks passing.
    Running,
    /// A tunnel dropped; rebuilding.
    Reconnecting,
    /// SSO expired and non-interactive login failed — user action required.
    NeedsLogin,
    /// Something went wrong; carries a short human-readable reason.
    Error(String),
}

/// Commands sent from the UI thread to the worker.
#[derive(Clone, Debug)]
pub enum Cmd {
    Start,
    Stop,
    /// Toggle write (primary) vs replica targets. Applies on next Start.
    SetWriteMode(bool),
    Quit,
}

/// Messages sent from the worker back to the UI thread.
#[derive(Clone, Debug)]
pub enum Event {
    /// A state transition — update the icon color and menu text.
    Status(Status),
    /// Write mode changed under the worker's control (the 30-min auto-revert),
    /// so the UI checkbox must follow.
    WriteMode(bool),
}

#[derive(Clone, Copy, Debug)]
enum ServiceType {
    Redis,
    Postgres,
}

#[derive(new)]
struct Tunnel {
    service: String,
    port: u16,
    #[new(default)]
    child: Option<Child>,
}

impl Tunnel {
    /// Spawn the `b2c2 tunnel` subprocess. Returns Err instead of panicking so
    /// the supervisor can turn a spawn failure into a visible Error state.
    fn start(&mut self) -> anyhow::Result<()> {
        let child = Command::new("b2c2")
            .args(["tunnel", "-e", "prod", "-s", &self.service, "-p"])
            .arg(self.port.to_string())
            .stdout(log_stdio())
            .stderr(log_stdio())
            .spawn()?;
        self.child = Some(child);
        Ok(())
    }

    fn kill(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// True while the subprocess is still alive.
    fn alive(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }
}

#[derive(new)]
struct Service {
    service_type: ServiceType,
    port: u16,
}

impl Service {
    /// Health check: returns true if the service answers within 10s.
    fn ping(&self) -> bool {
        let mut child = match self.spawn() {
            Ok(c) => c,
            Err(_) => return false,
        };
        match child.wait_timeout(Duration::from_secs(10)) {
            Ok(Some(status)) => status.success(),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                false
            }
        }
    }

    fn spawn(&self) -> std::io::Result<Child> {
        match self.service_type {
            ServiceType::Postgres => Command::new("psql")
                .args(["-h", "localhost", "-p"])
                .arg(self.port.to_string())
                .args(["-c", "SELECT 1"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn(),
            ServiceType::Redis => Command::new("redis-cli")
                .arg("-p")
                .arg(self.port.to_string())
                .args(["-t", "10", "ping"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn(),
        }
    }
}

fn build_specs(write_mode: bool) -> Vec<(String, u16, ServiceType)> {
    let suffix = if write_mode { "" } else { "replica" };
    vec![
        (format!("primaryredis{suffix}"), 6001, ServiceType::Redis),
        (format!("secondaryredis{suffix}"), 6002, ServiceType::Redis),
        (format!("optionsredis{suffix}"), 6003, ServiceType::Redis),
        ("optionsconfigdb".to_string(), 5608, ServiceType::Postgres),
    ]
}

/// Kill any lingering SSM sessions from a previous run.
fn kill_tunnels() {
    let _ = Command::new("killall")
        .arg("session-manager-plugin")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn is_logged_in() -> bool {
    match Command::new("b2c2")
        .args(["aws", "auth", "status"])
        .output()
    {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stderr);
            !text.contains("not logged in")
        }
        // If we can't even run b2c2, treat as not logged in so we surface it.
        Err(_) => false,
    }
}

/// Run interactive `b2c2 aws login` (opens a browser in the GUI session).
fn login() -> bool {
    matches!(
        Command::new("b2c2").args(["aws", "login"]).status(),
        Ok(status) if status.success()
    )
}

/// Path of the log file the tunnel subprocesses write to.
pub fn log_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join("Library/Logs/Tunnel/tunnel.log")
}

fn open_log() -> Option<File> {
    let path = log_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    OpenOptions::new().create(true).append(true).open(path).ok()
}

/// Stdio target for tunnel subprocesses: the log file, or /dev/null on failure.
fn log_stdio() -> Stdio {
    open_log().map(Stdio::from).unwrap_or_else(Stdio::null)
}

/// Post a native macOS notification (best-effort).
pub fn notify(title: &str, message: &str) {
    let script = format!("display notification {:?} with title {:?}", message, title);
    let _ = Command::new("osascript")
        .args(["-e", &script])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// What ended a monitoring session.
enum SessionEnd {
    Stopped,
    Quit,
    Restart(bool),  // write-mode changed; rebuild with this write_mode
    WriteTimeout,   // write mode hit its TTL; revert to read-only
    Failed(String), // tunnels/pings dropped
}

/// The worker entry point. Blocks forever handling commands.
///
/// `report` is called on every state transition (and on the write-mode
/// auto-revert) so the UI can update the icon, menu text, and checkbox.
pub fn run<R: Fn(Event)>(cmd_rx: Receiver<Cmd>, report: R) {
    let mut write_mode = false;
    // Absolute instant at which write mode reverts to read-only. Held across
    // reconnects so a dropped tunnel doesn't reset the 30-minute clock.
    let mut write_deadline: Option<Instant> = None;

    'idle: loop {
        report(Event::Status(Status::Stopped));
        // Block until told to start (or quit).
        loop {
            match cmd_rx.recv() {
                Ok(Cmd::Start) => break,
                Ok(Cmd::SetWriteMode(w)) => write_mode = w,
                Ok(Cmd::Stop) => {}
                Ok(Cmd::Quit) | Err(_) => return,
            }
        }

        // Supervised session: keep tunnels alive, re-auth and reconnect as
        // needed, until the user stops or write-mode changes.
        let mut backoff = 1u64;
        'session: loop {
            // Arm (or disarm) the write-mode TTL. Only set it when not already
            // armed, so reconnects keep counting down from the original start.
            match (write_mode, write_deadline) {
                (true, None) => write_deadline = Some(Instant::now() + WRITE_TTL),
                (false, _) => write_deadline = None,
                _ => {}
            }

            report(Event::Status(Status::Starting));
            kill_tunnels();

            // Ensure we're authenticated.
            if !is_logged_in() {
                report(Event::Status(Status::LoggingIn));
                notify("Tunnel", "AWS session expired — opening login…");
                if !login() || !is_logged_in() {
                    report(Event::Status(Status::NeedsLogin));
                    notify(
                        "Tunnel",
                        "AWS login required. Open the menu and press Start.",
                    );
                    // Give up this session; wait for a fresh command.
                    continue 'idle;
                }
            }

            match start_and_monitor(write_mode, write_deadline, &cmd_rx, &report) {
                SessionEnd::Stopped => continue 'idle,
                SessionEnd::Quit => return,
                SessionEnd::Restart(w) => {
                    write_mode = w;
                    backoff = 1;
                    continue 'session;
                }
                SessionEnd::WriteTimeout => {
                    write_mode = false;
                    write_deadline = None;
                    report(Event::WriteMode(false));
                    notify(
                        "Tunnel",
                        "Write mode expired after 30 min — reverting to read-only.",
                    );
                    backoff = 1;
                    continue 'session;
                }
                SessionEnd::Failed(reason) => {
                    report(Event::Status(Status::Reconnecting));
                    notify(
                        "Tunnel",
                        &format!("Connection lost ({reason}); reconnecting…"),
                    );
                    // Interruptible backoff so Stop/Quit stay responsive.
                    match sleep_watching(&cmd_rx, backoff) {
                        Some(SessionEnd::Stopped) => continue 'idle,
                        Some(SessionEnd::Quit) => return,
                        Some(SessionEnd::Restart(w)) => {
                            write_mode = w;
                            backoff = 1;
                            continue 'session;
                        }
                        _ => {}
                    }
                    backoff = (backoff * 2).min(30);
                    continue 'session;
                }
            }
        }
    }
}

/// Start all tunnels, then poll health + commands until something ends it.
fn start_and_monitor<R: Fn(Event)>(
    write_mode: bool,
    write_deadline: Option<Instant>,
    cmd_rx: &Receiver<Cmd>,
    report: &R,
) -> SessionEnd {
    let specs = build_specs(write_mode);
    let mut tunnels: Vec<Tunnel> = Vec::new();
    let mut services: Vec<Service> = Vec::new();
    for (service, port, kind) in specs {
        let mut tunnel = Tunnel::new(service, port);
        if let Err(e) = tunnel.start() {
            for mut t in tunnels {
                t.kill();
            }
            report(Event::Status(Status::Error(format!(
                "failed to launch b2c2: {e}"
            ))));
            notify("Tunnel", "Failed to launch b2c2 tunnel. Is b2c2 on PATH?");
            return SessionEnd::Failed("spawn failed".into());
        }
        tunnels.push(tunnel);
        services.push(Service::new(kind, port));
    }

    // Give the tunnels time to come up before the first health check.
    if let Some(end) = sleep_watching(cmd_rx, 20) {
        cleanup(&mut tunnels);
        return end;
    }
    report(Event::Status(Status::Running));

    loop {
        // Check for commands first so Stop/Quit are responsive.
        match cmd_rx.try_recv() {
            Ok(Cmd::Stop) => {
                cleanup(&mut tunnels);
                return SessionEnd::Stopped;
            }
            Ok(Cmd::Quit) => {
                cleanup(&mut tunnels);
                return SessionEnd::Quit;
            }
            Ok(Cmd::SetWriteMode(w)) => {
                cleanup(&mut tunnels);
                return SessionEnd::Restart(w);
            }
            Ok(Cmd::Start) | Err(_) => {}
        }

        // Write mode expired?
        if write_deadline.is_some_and(|dl| Instant::now() >= dl) {
            cleanup(&mut tunnels);
            return SessionEnd::WriteTimeout;
        }

        // Any tunnel process died?
        if tunnels.iter_mut().any(|t| !t.alive()) {
            cleanup(&mut tunnels);
            return SessionEnd::Failed("tunnel process exited".into());
        }
        // All health checks passing?
        if !services.iter().all(Service::ping) {
            cleanup(&mut tunnels);
            return SessionEnd::Failed("health check failed".into());
        }

        if let Some(end) = sleep_watching(cmd_rx, 5) {
            cleanup(&mut tunnels);
            return end;
        }
    }
}

fn cleanup(tunnels: &mut [Tunnel]) {
    for tunnel in tunnels.iter_mut() {
        tunnel.kill();
    }
    kill_tunnels();
}

/// Sleep for `secs`, but return early with a SessionEnd if a command arrives.
fn sleep_watching(cmd_rx: &Receiver<Cmd>, secs: u64) -> Option<SessionEnd> {
    let deadline = Duration::from_secs(secs);
    let step = Duration::from_millis(500);
    let mut elapsed = Duration::ZERO;
    while elapsed < deadline {
        match cmd_rx.recv_timeout(step) {
            Ok(Cmd::Stop) => return Some(SessionEnd::Stopped),
            Ok(Cmd::Quit) => return Some(SessionEnd::Quit),
            Ok(Cmd::SetWriteMode(w)) => return Some(SessionEnd::Restart(w)),
            Ok(Cmd::Start) => {}
            Err(RecvTimeoutError::Timeout) => elapsed += step,
            Err(RecvTimeoutError::Disconnected) => return Some(SessionEnd::Quit),
        }
    }
    None
}
