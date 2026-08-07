use std::{path::Path, sync::Arc, time::Duration};

use tokio::{
    net::{UnixStream, unix::OwnedWriteHalf},
    sync::{Mutex, mpsc},
    task::JoinHandle,
    time::sleep,
};

use super::client::HostClient;
use super::timeout::{CONNECT_RETRY_DELAY, CONNECT_TIMEOUT};
use crate::rpc::{JsonLineRpcPeer, RPC_EVENT_BUFFER, RpcError, RpcEvent};

pub struct HostRuntime {
    pub client: HostClient,
    pub events: HostEventReceiver,
    pub guard: HostConnectionGuard,
}

impl HostRuntime {
    pub async fn connect(socket_path: &Path, request_timeout: Duration) -> Result<Self, RpcError> {
        let stream = connect_with_retry(socket_path).await?;
        let (reader, writer) = stream.into_split();
        let writer = Arc::new(Mutex::new(Some(writer)));
        let peer = JsonLineRpcPeer::new(writer.clone(), "nabla-host-", request_timeout);
        let (event_tx, events) = mpsc::channel(RPC_EVENT_BUFFER);

        let read_peer = peer.clone();
        let read_task = tokio::spawn(async move {
            read_peer.read_from(reader, event_tx).await;
        });

        Ok(Self {
            client: HostClient::new(peer, request_timeout),
            events: HostEventReceiver { receiver: events },
            guard: HostConnectionGuard {
                writer,
                read_task: Some(read_task),
            },
        })
    }
}

pub struct HostEventReceiver {
    receiver: mpsc::Receiver<Result<RpcEvent, RpcError>>,
}

impl HostEventReceiver {
    pub async fn recv(&mut self) -> Option<Result<RpcEvent, RpcError>> {
        self.receiver.recv().await
    }
}

pub struct HostConnectionGuard {
    writer: Arc<Mutex<Option<OwnedWriteHalf>>>,
    read_task: Option<JoinHandle<()>>,
}

impl HostConnectionGuard {
    pub async fn shutdown(&mut self) {
        self.writer.lock().await.take();
        if let Some(task) = self.read_task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for HostConnectionGuard {
    fn drop(&mut self) {
        if let Some(task) = self.read_task.take() {
            task.abort();
        }
    }
}

async fn connect_with_retry(socket_path: &Path) -> Result<UnixStream, RpcError> {
    let started = tokio::time::Instant::now();
    loop {
        match UnixStream::connect(socket_path).await {
            Ok(stream) => return Ok(stream),
            Err(error) if started.elapsed() < CONNECT_TIMEOUT => {
                let _ = error;
                sleep(CONNECT_RETRY_DELAY).await;
            }
            Err(error) => {
                return Err(RpcError::Io(format!(
                    "failed to connect to host control socket {}: {error}",
                    socket_path.display()
                )));
            }
        }
    }
}
