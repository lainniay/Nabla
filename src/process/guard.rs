use std::{path::PathBuf, process::ExitStatus, sync::Arc, time::Duration};

use tokio::{
    process::{Child, ChildStdin},
    sync::{Mutex, mpsc},
    task::JoinHandle,
    time::timeout,
};

use crate::host::HostConnectionGuard;
use crate::rpc::RpcError;

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

pub struct PiChildGuard {
    pub(crate) child: Option<Child>,
    pub(crate) stdin: Arc<Mutex<Option<ChildStdin>>>,
    pub(crate) stderr: mpsc::Receiver<String>,
    pub(crate) stdout_task: Option<JoinHandle<()>>,
    pub(crate) stderr_task: Option<JoinHandle<()>>,
    pub(crate) host_guard: Option<HostConnectionGuard>,
    pub(crate) control_socket: PathBuf,
    pub(crate) session_dir: Option<PathBuf>,
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
