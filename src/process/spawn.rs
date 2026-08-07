use std::{process::Stdio, sync::Arc};

use tokio::{
    process::Command,
    sync::{Mutex, mpsc},
};

use crate::host::{HostClient, HostEventReceiver, HostRuntime};
use crate::pi::{PiClient, PiEventReceiver};
use crate::process::config::PiProcessConfig;
use crate::process::guard::PiChildGuard;
use crate::process::stderr::read_stderr;
use crate::rpc::{JsonLineRpcPeer, RPC_EVENT_BUFFER, RpcError};

pub struct PiRuntime {
    pub client: PiClient,
    pub events: PiEventReceiver,
    pub host: HostClient,
    pub host_events: HostEventReceiver,
    pub process: PiChildGuard,
}

impl PiRuntime {
    pub async fn spawn(config: PiProcessConfig) -> Result<Self, RpcError> {
        let mut command = Command::new(&config.executable);
        let nabla_executable =
            std::env::current_exe().map_err(|error| RpcError::Spawn(error.to_string()))?;
        command
            .args(&config.args)
            .current_dir(&config.cwd)
            .env("NABLA_CONTROL_SOCKET", &config.control_socket)
            .env("NABLA_EXECUTABLE", &nabla_executable)
            // Node reads this switch during process startup. Setting it here
            // makes Pi's global fetch honor HTTP_PROXY/HTTPS_PROXY/NO_PROXY.
            .env("NODE_USE_ENV_PROXY", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if config.offline {
            command.env("PI_OFFLINE", "1");
        }
        if let Some(session_dir) = config.session_dir.as_ref() {
            command.env("PI_CODING_AGENT_SESSION_DIR", session_dir);
        }

        let mut child = command
            .spawn()
            .map_err(|error| RpcError::Spawn(error.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| RpcError::Spawn("Pi stdin was not piped".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| RpcError::Spawn("Pi stdout was not piped".to_owned()))?;
        let child_stderr = child
            .stderr
            .take()
            .ok_or_else(|| RpcError::Spawn("Pi stderr was not piped".to_owned()))?;

        let stdin = Arc::new(Mutex::new(Some(stdin)));
        let peer = JsonLineRpcPeer::new(stdin.clone(), "nabla-", config.request_timeout);
        let (event_tx, events) = mpsc::channel(RPC_EVENT_BUFFER);
        let (stderr_tx, mut stderr) = mpsc::channel(256);

        let stdout_peer = peer.clone();
        let stdout_task = tokio::spawn(async move {
            stdout_peer.read_from(stdout, event_tx).await;
        });
        let stderr_task = tokio::spawn(async move {
            read_stderr(child_stderr, stderr_tx).await;
        });

        let HostRuntime {
            client: host,
            events: host_events,
            guard: host_guard,
        } = match HostRuntime::connect(&config.control_socket, config.request_timeout).await {
            Ok(runtime) => runtime,
            Err(error) => {
                let diagnostics = std::iter::from_fn(|| stderr.try_recv().ok())
                    .take(8)
                    .collect::<Vec<_>>();
                let _ = child.kill().await;
                stdout_task.abort();
                stderr_task.abort();
                let _ = std::fs::remove_file(&config.control_socket);
                if let Some(session_dir) = config.session_dir.as_ref() {
                    let _ = std::fs::remove_dir_all(session_dir);
                }
                if diagnostics.is_empty() {
                    return Err(error);
                }
                return Err(RpcError::Io(format!(
                    "{error}; host stderr: {}",
                    diagnostics.join(" | ")
                )));
            }
        };

        Ok(Self {
            client: PiClient { peer },
            events: PiEventReceiver { receiver: events },
            host,
            host_events,
            process: PiChildGuard {
                child: Some(child),
                stdin,
                stderr,
                stdout_task: Some(stdout_task),
                stderr_task: Some(stderr_task),
                host_guard: Some(host_guard),
                control_socket: config.control_socket,
                session_dir: config.session_dir,
            },
        })
    }
}
