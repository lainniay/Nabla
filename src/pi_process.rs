use std::{
    path::PathBuf,
    process::{ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{Mutex, mpsc},
    task::JoinHandle,
    time::timeout,
};

use crate::{
    file_references::ImageContent,
    host::{HostClient, HostConnectionGuard, HostEventReceiver, HostRuntime},
    rpc::{
        DEFAULT_REQUEST_TIMEOUT, JsonLineRpcPeer, PiState, RPC_EVENT_BUFFER, RpcError, RpcEvent,
        RpcResponse,
    },
};

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
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
        config.session_dir = Some(PathBuf::from("/tmp").join(format!("{socket_name}-sessions")));
        config
    }

    fn local_base(cwd: PathBuf) -> Self {
        let host = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("agent-host")
            .join("src")
            .join("main.ts");
        let socket_id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
        let control_socket =
            PathBuf::from("/tmp").join(format!("nabla-{}-{socket_id}.sock", std::process::id()));

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

/// Complete runtime split into a cloneable command client, an exclusive event
/// receiver, and an owning child-process guard.
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
        command
            .args(&config.args)
            .current_dir(&config.cwd)
            .env("NABLA_CONTROL_SOCKET", &config.control_socket)
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

/// Cloneable handle for commands sent to Pi over stdin.
#[derive(Clone)]
pub struct PiClient {
    peer: JsonLineRpcPeer<ChildStdin>,
}

impl PiClient {
    pub async fn get_state(&self) -> Result<PiState, RpcError> {
        self.request_data("get_state", Map::new()).await
    }

    pub async fn prompt(
        &self,
        message: impl Into<String>,
        images: Option<Vec<ImageContent>>,
    ) -> Result<(), RpcError> {
        self.deliver("prompt", message.into(), images).await
    }

    pub async fn steer(
        &self,
        message: impl Into<String>,
        images: Option<Vec<ImageContent>>,
    ) -> Result<(), RpcError> {
        self.deliver("steer", message.into(), images).await
    }

    pub async fn follow_up(
        &self,
        message: impl Into<String>,
        images: Option<Vec<ImageContent>>,
    ) -> Result<(), RpcError> {
        self.deliver("follow_up", message.into(), images).await
    }

    async fn deliver(
        &self,
        command: &str,
        message: String,
        images: Option<Vec<ImageContent>>,
    ) -> Result<(), RpcError> {
        let mut parameters = Map::new();
        parameters.insert("message".to_owned(), Value::String(message));
        if let Some(images) = images.filter(|images| !images.is_empty()) {
            parameters.insert(
                "images".to_owned(),
                serde_json::to_value(images).map_err(|error| RpcError::Json(error.to_string()))?,
            );
        }
        self.request(command, parameters).await?.ensure_success()
    }

    pub async fn abort(&self) -> Result<(), RpcError> {
        self.request("abort", Map::new()).await?.ensure_success()
    }

    pub async fn compact(&self, custom_instructions: Option<String>) -> Result<Value, RpcError> {
        let mut parameters = Map::new();
        if let Some(instructions) = custom_instructions {
            parameters.insert("customInstructions".to_owned(), Value::String(instructions));
        }
        self.request_data("compact", parameters).await
    }

    pub async fn request_data<T: DeserializeOwned>(
        &self,
        command: &str,
        parameters: Map<String, Value>,
    ) -> Result<T, RpcError> {
        self.peer.request_data(command, parameters).await
    }

    pub async fn request(
        &self,
        command: &str,
        parameters: Map<String, Value>,
    ) -> Result<RpcResponse, RpcError> {
        self.peer.request(command, parameters).await
    }
}

/// Exclusive receiver for asynchronous agent lifecycle and streaming events.
pub struct PiEventReceiver {
    receiver: mpsc::Receiver<Result<RpcEvent, RpcError>>,
}

impl PiEventReceiver {
    pub async fn recv(&mut self) -> Option<Result<RpcEvent, RpcError>> {
        self.receiver.recv().await
    }
}

/// Owns the operating-system process and guarantees eventual termination.
pub struct PiChildGuard {
    child: Option<Child>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    stderr: mpsc::Receiver<String>,
    stdout_task: Option<JoinHandle<()>>,
    stderr_task: Option<JoinHandle<()>>,
    host_guard: Option<HostConnectionGuard>,
    control_socket: PathBuf,
    session_dir: Option<PathBuf>,
}

impl PiChildGuard {
    pub async fn recv_stderr(&mut self) -> Option<String> {
        self.stderr.recv().await
    }

    pub async fn shutdown(&mut self) -> Result<ExitStatus, RpcError> {
        if let Some(mut host_guard) = self.host_guard.take() {
            host_guard.shutdown().await;
        }
        self.stdin.lock().await.take();
        let child = self.child.as_mut().ok_or(RpcError::ProcessExited)?;

        let status = match timeout(SHUTDOWN_TIMEOUT, child.wait()).await {
            Ok(result) => result.map_err(|error| RpcError::Io(error.to_string()))?,
            Err(_) => {
                child
                    .kill()
                    .await
                    .map_err(|error| RpcError::Io(error.to_string()))?;
                child
                    .wait()
                    .await
                    .map_err(|error| RpcError::Io(error.to_string()))?
            }
        };

        self.child.take();
        if let Some(task) = self.stdout_task.take() {
            let _ = task.await;
        }
        if let Some(task) = self.stderr_task.take() {
            let _ = task.await;
        }
        let _ = std::fs::remove_file(&self.control_socket);
        if let Some(session_dir) = self.session_dir.take() {
            let _ = std::fs::remove_dir_all(session_dir);
        }

        Ok(status)
    }
}

impl Drop for PiChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
        if let Some(task) = self.stdout_task.take() {
            task.abort();
        }
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }
        self.host_guard.take();
        let _ = std::fs::remove_file(&self.control_socket);
        if let Some(session_dir) = self.session_dir.take() {
            let _ = std::fs::remove_dir_all(session_dir);
        }
    }
}

async fn read_stderr(stderr: tokio::process::ChildStderr, stderr_tx: mpsc::Sender<String>) {
    let mut lines = BufReader::new(stderr).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                if stderr_tx.send(line).await.is_err() {
                    break;
                }
            }
            Ok(None) => break,
            Err(error) => {
                let _ = stderr_tx
                    .send(format!("failed reading Pi stderr: {error}"))
                    .await;
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{SessionScope, SessionSortMode, TreeFilterMode};

    #[test]
    fn local_configs_use_repo_local_host_and_unique_control_sockets() {
        let local = PiProcessConfig::local(PathBuf::from("/tmp/project"));
        let smoke = PiProcessConfig::local_smoke(PathBuf::from("/tmp/project"));

        assert_eq!(local.executable, PathBuf::from("node"));
        assert!(
            local
                .args
                .first()
                .is_some_and(|arg| arg.ends_with("agent-host/src/main.ts"))
        );
        assert_eq!(local.cwd, PathBuf::from("/tmp/project"));
        assert!(local.control_socket.starts_with("/tmp"));
        assert_ne!(local.control_socket, smoke.control_socket);
        assert!(local.session_dir.is_none());
        assert!(smoke.session_dir.is_some());
        assert!(!local.offline);
        assert!(smoke.offline);
    }

    #[test]
    fn delimiter_stripping_uses_lf_and_optional_cr_only() {
        let mut lf = "hello\n".to_owned();
        crate::rpc::strip_record_delimiter(&mut lf);
        assert_eq!(lf, "hello");

        let mut crlf = "hello\r\n".to_owned();
        crate::rpc::strip_record_delimiter(&mut crlf);
        assert_eq!(crlf, "hello");

        let mut unicode = "hello\u{2028}world".to_owned();
        crate::rpc::strip_record_delimiter(&mut unicode);
        assert_eq!(unicode, "hello\u{2028}world");
    }

    #[tokio::test]
    async fn repo_local_pi_get_state_smoke() {
        let cwd = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let config = PiProcessConfig::local_smoke(cwd.clone());
        let session_dir = config.session_dir.clone().unwrap();
        if !config
            .args
            .last()
            .is_some_and(|arg| PathBuf::from(arg).exists())
        {
            return;
        }

        let runtime = match PiRuntime::spawn(config).await {
            Ok(runtime) => runtime,
            Err(error) if error.to_string().contains("listen EPERM") => {
                // Some CI/sandbox profiles deny Unix-domain socket creation to
                // child processes. The protocol is still covered by unit tests;
                // run this smoke path when the platform permits the socket.
                return;
            }
            Err(error) => panic!("failed to start repository-local Pi: {error}"),
        };
        let PiRuntime {
            client,
            events: _events,
            host,
            host_events: _host_events,
            mut process,
        } = runtime;
        let state = client.get_state().await.unwrap();
        assert!(host.get_plan_state().await.unwrap().artifact.is_none());
        let context = host.get_context_state().await.unwrap();
        assert!(context.policy.enabled);
        assert_eq!(context.policy.recent_tool_result_tokens, 40_000);
        assert_eq!(context.estimated_cumulative_avoided_tokens, 0);
        let resources = host.get_resources().await.unwrap();
        assert_eq!(resources.revision, 1);
        assert!(
            resources
                .extensions
                .iter()
                .any(|path| path.contains("nabla-control"))
        );
        assert!(host.get_goal().await.unwrap().goal.is_none());
        let agents = host.get_agents().await.unwrap();
        assert!(
            agents
                .profiles
                .iter()
                .any(|profile| profile.name == "reviewer")
        );
        let reloaded_agents = host.reload_agents().await.unwrap();
        assert_eq!(reloaded_agents.profiles.len(), agents.profiles.len());
        assert!(
            host.start_subagent("missing".to_owned(), "task".to_owned())
                .await
                .is_err()
        );
        assert!(host.clear_queue().await.unwrap().restored_text.is_empty());
        let providers = host.list_providers().await.unwrap();
        let standard = host.set_plan_mode(false).await.unwrap();
        assert!(!standard.active);
        assert_eq!(
            standard.active_tools,
            [
                "read",
                "grep",
                "find",
                "ls",
                "edit",
                "write",
                "bash",
                "delegate_task"
            ]
        );
        let plan = host.set_plan_mode(true).await.unwrap();
        assert!(plan.active);
        assert_eq!(
            plan.active_tools,
            [
                "read",
                "grep",
                "find",
                "ls",
                "ask_user",
                "submit_plan",
                "delegate_task"
            ]
        );
        let standard = host.set_plan_mode(false).await.unwrap();
        assert!(!standard.active);
        assert_eq!(
            standard.active_tools,
            [
                "read",
                "grep",
                "find",
                "ls",
                "edit",
                "write",
                "bash",
                "delegate_task"
            ]
        );
        let state_after_modes = client.get_state().await.unwrap();
        assert_eq!(state_after_modes.session_id, state.session_id);
        assert!(!state.session_id.is_empty());
        assert!(!providers.is_empty());

        let browser = host.open_session_browser().await.unwrap();
        let all_sessions = host
            .query_session_browser(
                browser.browser_id.clone(),
                SessionScope::All,
                String::new(),
                SessionSortMode::Recent,
                false,
                0,
            )
            .await
            .unwrap();
        assert_eq!(all_sessions.scope, SessionScope::All);
        host.close_session_browser(browser.browser_id)
            .await
            .unwrap();

        let tree = host
            .get_tree_state(TreeFilterMode::Default, String::new(), Vec::new())
            .await
            .unwrap();
        assert!(tree.items.is_empty());

        let new_session = host.new_session().await.unwrap();
        assert!(!new_session.cancelled);
        let activation = new_session.activation.expect("new session activation");
        assert!(!activation.state.session_id.is_empty());
        assert_eq!(
            client.get_state().await.unwrap().session_id,
            activation.state.session_id
        );

        let status = process.shutdown().await.unwrap();
        assert!(status.success());
        assert!(!session_dir.exists());
    }
}
