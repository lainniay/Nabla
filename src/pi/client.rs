use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use tokio::process::ChildStdin;

use crate::file_references::ImageContent;
use crate::rpc::{JsonLineRpcPeer, PiState, RpcError, RpcResponse};

#[derive(Clone)]
pub struct PiClient {
    pub(crate) peer: JsonLineRpcPeer<ChildStdin>,
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
