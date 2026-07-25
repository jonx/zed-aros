//! Crash reporting on AROS.
//!
//! The real implementation captures a minidump in a subprocess and ships it
//! over an IPC socket (`crash-handler` + `minidumper`). Neither builds for
//! AROS: they need per-platform exception handling and a unix-socket IPC
//! channel. AROS has its own trap handler, which already prints a symbolised
//! backtrace to the debug log, so nothing is lost by not duplicating it here.
//!
//! This module mirrors the crate's public API exactly so callers need no
//! `cfg`s; every entry point is inert.

use serde::{Deserialize, Serialize};
use std::panic::Location;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use system_specs::GpuSpecs;

/// Stands in for `minidumper::Client`. Never connected to anything.
pub struct Client;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CrashInfo {
    pub init: InitCrashHandler,
    pub panic: Option<CrashPanic>,
    pub minidump_error: Option<String>,
    pub gpus: Vec<system_specs::GpuInfo>,
    pub active_gpu: Option<system_specs::GpuSpecs>,
    pub user_info: Option<UserInfo>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct InitCrashHandler {
    pub session_id: String,
    pub zed_version: String,
    pub binary: String,
    pub release_channel: String,
    pub commit_sha: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct CrashPanic {
    pub message: String,
    pub span: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct UserInfo {
    pub metrics_id: Option<String>,
    pub is_staff: Option<bool>,
}

/// The real version sets `RUST_BACKTRACE`; AROS's trap handler always prints a
/// backtrace, so there is nothing to force.
pub fn force_backtrace() {}

pub fn init<F, S, C, P>(
    _crash_init: InitCrashHandler,
    _spawn: S,
    _socket_path: P,
    _wait_timer: C,
) -> impl Future<Output = Arc<Client>> + use<F, C, S, P>
where
    F: Future<Output = ()> + Send + Sync + 'static,
    C: (Fn(Duration) -> F) + Send + Sync + 'static,
    S: FnOnce(Pin<Box<dyn Future<Output = ()> + Send + 'static>>),
    P: FnOnce(u32) -> PathBuf,
{
    async { Arc::new(Client) }
}

pub fn set_gpu_info(_crash_client: &Arc<Client>, _specs: GpuSpecs) {}

pub fn set_user_info(_crash_client: &Arc<Client>, _info: UserInfo) {}

pub fn panic_hook(_crash_client: Arc<Client>, _message: &str, _location: Option<&Location>) {}

pub fn crash_server(_socket: &Path, _logs_dir: PathBuf) {}
