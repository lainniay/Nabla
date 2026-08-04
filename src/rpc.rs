use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    sync::{Mutex, mpsc, oneshot},
    time::timeout,
};

// INFO: Keep one shared timeout policy for request/response correlation.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
pub const RPC_EVENT_BUFFER: usize = 2_048;

#[derive(Debug, Clone, Error)]
pub enum RpcError {
    #[error("failed to spawn Pi: {0}")]
    Spawn(String),
    #[error("Pi RPC I/O failed: {0}")]
    Io(String),
    #[error("invalid Pi RPC JSON: {0}")]
    Json(String),
    #[error("invalid Pi RPC message: {0}")]
    Protocol(String),
    #[error("Pi RPC request {id} timed out")]
    Timeout { id: String },
    #[error("Pi RPC process exited")]
    ProcessExited,
    #[error("Pi RPC response channel closed for request {id}")]
    ResponseChannelClosed { id: String },
    #[error("Pi command {command} failed: {message}")]
    Remote { command: String, message: String },
    #[error("invalid response data for Pi command {command}: {message}")]
    InvalidData { command: String, message: String },
}

// INFO: Outbound requests flatten command parameters to match Pi's JSONL protocol.
#[derive(Debug, Clone, Serialize)]
pub struct RpcRequest {
    pub id: String,
    #[serde(rename = "type")]
    pub command: String,
    #[serde(flatten)]
    pub parameters: Map<String, Value>,
}

impl RpcRequest {
    pub fn new(
        id: impl Into<String>,
        command: impl Into<String>,
        parameters: Map<String, Value>,
    ) -> Self {
        Self {
            id: id.into(),
            command: command.into(),
            parameters,
        }
    }

    pub fn get_state(id: impl Into<String>) -> Self {
        Self::new(id, "get_state", Map::new())
    }
}

// INFO: Responses retain the command name so decoding failures remain actionable.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RpcResponse {
    pub id: Option<String>,
    pub command: String,
    pub success: bool,
    #[serde(default)]
    pub data: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

impl RpcResponse {
    pub fn ensure_success(self) -> Result<(), RpcError> {
        if self.success {
            return Ok(());
        }

        Err(RpcError::Remote {
            command: self.command,
            message: self
                .error
                .unwrap_or_else(|| "unknown remote error".to_owned()),
        })
    }

    pub fn into_data<T: DeserializeOwned>(self) -> Result<T, RpcError> {
        if !self.success {
            return Err(RpcError::Remote {
                command: self.command,
                message: self
                    .error
                    .unwrap_or_else(|| "unknown remote error".to_owned()),
            });
        }

        let data = self.data.unwrap_or(Value::Null);
        serde_json::from_value(data).map_err(|error| RpcError::InvalidData {
            command: self.command,
            message: error.to_string(),
        })
    }
}

// INFO: Unsolicited events bypass request correlation and enter the app reducer.
#[derive(Debug, Clone)]
pub struct RpcEvent {
    pub kind: String,
    pub payload: Value,
}

// pi -> rust info enum
#[derive(Debug, Clone)]
pub enum IncomingMessage {
    Response(RpcResponse),
    Event(RpcEvent),
}

type PendingRequest = oneshot::Sender<Result<RpcResponse, RpcError>>;
type PendingRequests = Arc<Mutex<HashMap<String, PendingRequest>>>;

/// Shared JSON-lines request/response transport used by both Pi and the local
/// host control socket. Domain clients deliberately remain separate facades.
pub struct JsonLineRpcPeer<W> {
    writer: Arc<Mutex<Option<W>>>,
    pending: PendingRequests,
    next_request_id: Arc<AtomicU64>,
    request_id_prefix: &'static str,
    request_timeout: Duration,
}

impl<W> Clone for JsonLineRpcPeer<W> {
    fn clone(&self) -> Self {
        Self {
            writer: self.writer.clone(),
            pending: self.pending.clone(),
            next_request_id: self.next_request_id.clone(),
            request_id_prefix: self.request_id_prefix,
            request_timeout: self.request_timeout,
        }
    }
}

impl<W> JsonLineRpcPeer<W> {
    pub fn new(
        writer: Arc<Mutex<Option<W>>>,
        request_id_prefix: &'static str,
        request_timeout: Duration,
    ) -> Self {
        Self {
            writer,
            pending: PendingRequests::default(),
            next_request_id: Arc::new(AtomicU64::new(1)),
            request_id_prefix,
            request_timeout,
        }
    }

    pub fn writer_handle(&self) -> Arc<Mutex<Option<W>>> {
        self.writer.clone()
    }

    pub async fn read_from<R>(&self, reader: R, event_tx: mpsc::Sender<Result<RpcEvent, RpcError>>)
    where
        R: AsyncRead + Unpin,
    {
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    self.fail_pending(RpcError::ProcessExited).await;
                    break;
                }
                Ok(_) => {
                    strip_record_delimiter(&mut line);
                    if line.is_empty() {
                        continue;
                    }

                    match parse_incoming_line(&line) {
                        Ok(IncomingMessage::Response(response)) => {
                            let sender = match response.id.as_ref() {
                                Some(id) => self.pending.lock().await.remove(id),
                                None => None,
                            };
                            if let Some(sender) = sender {
                                let _ = sender.send(Ok(response));
                            } else {
                                let payload =
                                    serde_json::to_value(&response).unwrap_or(Value::Null);
                                if !forward_event(
                                    &event_tx,
                                    Ok(RpcEvent {
                                        kind: "unmatched_response".to_owned(),
                                        payload,
                                    }),
                                )
                                .await
                                {
                                    break;
                                }
                            }
                        }
                        Ok(IncomingMessage::Event(event)) => {
                            if !forward_event(&event_tx, Ok(event)).await {
                                break;
                            }
                        }
                        Err(error) => {
                            if !forward_event(&event_tx, Err(error)).await {
                                break;
                            }
                        }
                    }
                }
                Err(error) => {
                    let error = RpcError::Io(error.to_string());
                    self.fail_pending(error.clone()).await;
                    let _ = forward_event(&event_tx, Err(error)).await;
                    break;
                }
            }
        }
    }

    async fn fail_pending(&self, error: RpcError) {
        let requests = std::mem::take(&mut *self.pending.lock().await);
        for (_, sender) in requests {
            let _ = sender.send(Err(error.clone()));
        }
    }
}

async fn forward_event(
    sender: &mpsc::Sender<Result<RpcEvent, RpcError>>,
    event: Result<RpcEvent, RpcError>,
) -> bool {
    match sender.try_send(event) {
        Ok(()) => true,
        Err(mpsc::error::TrySendError::Closed(_)) => false,
        Err(mpsc::error::TrySendError::Full(event)) => {
            if matches!(
                &event,
                Ok(event)
                    if matches!(
                        event.kind.as_str(),
                        "tool_execution_update" | "session_list_progress"
                    )
            ) {
                true
            } else {
                sender.send(event).await.is_ok()
            }
        }
    }
}

impl<W> JsonLineRpcPeer<W>
where
    W: AsyncWrite + Unpin,
{
    pub async fn request_data<T: DeserializeOwned>(
        &self,
        command: &str,
        parameters: Map<String, Value>,
    ) -> Result<T, RpcError> {
        self.request(command, parameters).await?.into_data()
    }

    pub async fn request_data_with_timeout<T: DeserializeOwned>(
        &self,
        command: &str,
        parameters: Map<String, Value>,
        request_timeout: Duration,
    ) -> Result<T, RpcError> {
        self.request_with_timeout(command, parameters, request_timeout)
            .await?
            .into_data()
    }

    pub async fn request(
        &self,
        command: &str,
        parameters: Map<String, Value>,
    ) -> Result<RpcResponse, RpcError> {
        self.request_with_timeout(command, parameters, self.request_timeout)
            .await
    }

    pub async fn request_with_timeout(
        &self,
        command: &str,
        parameters: Map<String, Value>,
        request_timeout: Duration,
    ) -> Result<RpcResponse, RpcError> {
        let id = format!(
            "{}{}",
            self.request_id_prefix,
            self.next_request_id.fetch_add(1, Ordering::Relaxed)
        );
        let request = RpcRequest::new(id.clone(), command, parameters);
        let encoded = encode_line(&request)?;
        let (response_tx, response_rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), response_tx);

        let write_result = async {
            let mut writer = self.writer.lock().await;
            let writer = writer.as_mut().ok_or(RpcError::ProcessExited)?;
            writer
                .write_all(&encoded)
                .await
                .map_err(|error| RpcError::Io(error.to_string()))?;
            writer
                .flush()
                .await
                .map_err(|error| RpcError::Io(error.to_string()))
        }
        .await;

        if let Err(error) = write_result {
            self.pending.lock().await.remove(&id);
            return Err(error);
        }

        match timeout(request_timeout, response_rx).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => Err(RpcError::ResponseChannelClosed { id }),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(RpcError::Timeout { id })
            }
        }
    }
}

pub fn strip_record_delimiter(line: &mut String) {
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
}

// Pistate of get_state
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PiState {
    pub model: Option<Value>,
    pub thinking_level: String,
    pub is_streaming: bool,
    pub is_compacting: bool,
    pub steering_mode: String,
    pub follow_up_mode: String,
    pub session_file: Option<String>,
    pub session_id: String,
    pub session_name: Option<String>,
    pub auto_compaction_enabled: bool,
    pub message_count: u64,
    pub pending_message_count: u64,
}

// Generic serialization function
pub fn encode_line<T: Serialize>(message: &T) -> Result<Vec<u8>, RpcError> {
    let mut encoded =
        serde_json::to_vec(message).map_err(|error| RpcError::Json(error.to_string()))?;
    encoded.push(b'\n');
    Ok(encoded)
}

// decode info from pi
pub fn parse_incoming_line(line: &str) -> Result<IncomingMessage, RpcError> {
    let payload: Value =
        serde_json::from_str(line).map_err(|error| RpcError::Json(error.to_string()))?;
    let object = payload
        .as_object()
        .ok_or_else(|| RpcError::Protocol("top-level message is not an object".to_owned()))?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::Protocol("message has no string `type` field".to_owned()))?;

    if kind == "response" {
        let response = serde_json::from_value(payload)
            .map_err(|error| RpcError::Protocol(error.to_string()))?;
        return Ok(IncomingMessage::Response(response));
    }

    Ok(IncomingMessage::Event(RpcEvent {
        kind: kind.to_owned(),
        payload,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, duplex, split};

    #[test]
    fn get_state_request_is_lf_delimited_json() {
        let encoded = encode_line(&RpcRequest::get_state("request-1")).unwrap();

        assert_eq!(
            encoded,
            br#"{"id":"request-1","type":"get_state"}
"#
        );
    }

    #[test]
    fn parser_keeps_unicode_line_separators_inside_json_strings() {
        let line = "{\"type\":\"notice\",\"text\":\"left\u{2028}right\"}";
        let IncomingMessage::Event(event) = parse_incoming_line(line).unwrap() else {
            panic!("expected event");
        };

        assert_eq!(event.kind, "notice");
        assert_eq!(event.payload["text"], "left\u{2028}right");
    }

    #[test]
    fn parses_success_response_and_state() {
        let line = r#"{"id":"request-1","type":"response","command":"get_state","success":true,"data":{"model":null,"thinkingLevel":"off","isStreaming":false,"isCompacting":false,"steeringMode":"one-at-a-time","followUpMode":"one-at-a-time","sessionId":"session-1","autoCompactionEnabled":true,"messageCount":0,"pendingMessageCount":0}}"#;
        let IncomingMessage::Response(response) = parse_incoming_line(line).unwrap() else {
            panic!("expected response");
        };

        assert_eq!(response.id.as_deref(), Some("request-1"));
        let state: PiState = response.into_data().unwrap();
        assert_eq!(state.session_id, "session-1");
        assert!(!state.is_streaming);
    }

    #[test]
    fn converts_failed_response_to_remote_error() {
        let response = RpcResponse {
            id: Some("request-1".to_owned()),
            command: "get_state".to_owned(),
            success: false,
            data: None,
            error: Some("not ready".to_owned()),
        };

        let error = response.into_data::<Value>().unwrap_err();
        assert!(matches!(
            error,
            RpcError::Remote {
                command,
                message
            } if command == "get_state" && message == "not ready"
        ));
    }

    #[tokio::test]
    async fn shared_transport_routes_responses_and_uses_the_configured_prefix() {
        let (client, server) = duplex(1024);
        let (client_reader, client_writer) = split(client);
        let peer = JsonLineRpcPeer::new(
            Arc::new(Mutex::new(Some(client_writer))),
            "test-peer-",
            Duration::from_secs(1),
        );
        let (event_tx, mut events) = mpsc::channel(RPC_EVENT_BUFFER);
        let read_peer = peer.clone();
        let read_task = tokio::spawn(async move {
            read_peer.read_from(client_reader, event_tx).await;
        });
        let server_task = tokio::spawn(async move {
            let (server_reader, mut server_writer) = split(server);
            let mut reader = BufReader::new(server_reader);
            let mut request = String::new();
            reader.read_line(&mut request).await.unwrap();
            let request: Value = serde_json::from_str(&request).unwrap();
            assert_eq!(request["id"], "test-peer-1");
            server_writer
                .write_all(
                    br#"{"id":"test-peer-1","type":"response","command":"ping","success":true,"data":{"answer":42}}
"#,
                )
                .await
                .unwrap();
        });

        let response: Value = peer.request_data("ping", Map::new()).await.unwrap();
        assert_eq!(response["answer"], 42);
        server_task.await.unwrap();
        read_task.await.unwrap();
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn shared_transport_exposes_unmatched_responses_consistently() {
        let (client, mut server) = duplex(1024);
        let (client_reader, client_writer) = split(client);
        let peer = JsonLineRpcPeer::new(
            Arc::new(Mutex::new(Some(client_writer))),
            "test-peer-",
            Duration::from_secs(1),
        );
        let (event_tx, mut events) = mpsc::channel(RPC_EVENT_BUFFER);
        let read_peer = peer.clone();
        let read_task = tokio::spawn(async move {
            read_peer.read_from(client_reader, event_tx).await;
        });
        server
            .write_all(
                br#"{"id":"unknown","type":"response","command":"late","success":true}
"#,
            )
            .await
            .unwrap();
        drop(server);

        let event = events.recv().await.unwrap().unwrap();
        assert_eq!(event.kind, "unmatched_response");
        assert_eq!(event.payload["command"], "late");
        read_task.await.unwrap();
    }

    #[tokio::test]
    async fn bounded_event_forwarding_drops_only_replaceable_updates() {
        let (sender, mut receiver) = mpsc::channel(1);
        sender
            .send(Ok(RpcEvent {
                kind: "agent_start".to_owned(),
                payload: Value::Null,
            }))
            .await
            .unwrap();

        assert!(
            forward_event(
                &sender,
                Ok(RpcEvent {
                    kind: "tool_execution_update".to_owned(),
                    payload: Value::Null,
                }),
            )
            .await
        );
        assert_eq!(receiver.recv().await.unwrap().unwrap().kind, "agent_start");
        assert!(receiver.try_recv().is_err());

        sender
            .send(Ok(RpcEvent {
                kind: "message_update".to_owned(),
                payload: Value::Null,
            }))
            .await
            .unwrap();
        let waiting_sender = sender.clone();
        let lifecycle = tokio::spawn(async move {
            forward_event(
                &waiting_sender,
                Ok(RpcEvent {
                    kind: "agent_end".to_owned(),
                    payload: Value::Null,
                }),
            )
            .await
        });
        assert_eq!(
            receiver.recv().await.unwrap().unwrap().kind,
            "message_update"
        );
        assert!(lifecycle.await.unwrap());
        assert_eq!(receiver.recv().await.unwrap().unwrap().kind, "agent_end");
    }
}
