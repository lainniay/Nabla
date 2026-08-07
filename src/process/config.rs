use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use crate::rpc::DEFAULT_REQUEST_TIMEOUT;

static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct PiProcessConfig {
    pub executable: PathBuf,
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub control_socket: PathBuf,
    pub session_dir: Option<PathBuf>,
    pub offline: bool,
    pub request_timeout: Duration,
}

impl PiProcessConfig {
    /// Local interactive configuration backed by the repository TypeScript host.
    // INFO: Each runtime gets a unique control socket to prevent cross-session
    // host events from reaching the wrong reducer.
    pub fn local(cwd: PathBuf) -> Self {
        Self::local_base(cwd)
    }

    /// Deterministic startup used by the headless get_state smoke test.
    pub fn local_smoke(cwd: PathBuf) -> Self {
        let mut config = Self::local_base(cwd);
        config.offline = true;
        let socket_name = config
            .control_socket
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("nabla-smoke");
        config.session_dir = Some(std::env::temp_dir().join(format!("{socket_name}-sessions")));
        config
    }

    fn local_base(cwd: PathBuf) -> Self {
        let host = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("agent-host")
            .join("src")
            .join("main.ts");
        let socket_id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
        let control_socket =
            std::env::temp_dir().join(format!("nabla-{}-{socket_id}.sock", std::process::id()));

        Self {
            executable: PathBuf::from("node"),
            cwd,
            args: vec![host.to_string_lossy().into_owned()],
            control_socket,
            session_dir: None,
            offline: false,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }
}
