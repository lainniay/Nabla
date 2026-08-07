use tokio::sync::mpsc;

use crate::rpc::{RpcError, RpcEvent};

pub struct PiEventReceiver {
    pub(crate) receiver: mpsc::Receiver<Result<RpcEvent, RpcError>>,
}

impl PiEventReceiver {
    pub async fn recv(&mut self) -> Option<Result<RpcEvent, RpcError>> {
        self.receiver.recv().await
    }
}
