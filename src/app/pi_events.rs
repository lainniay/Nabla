use super::*;

// INFO: Pi owns streaming/session truth; this reducer mirrors its lifecycle
// without synthesizing completion from request acknowledgements.
impl App {
    pub(super) fn update_pi(&mut self, event: RpcEvent) {
        let event_session = event.payload["sessionId"]
            .as_str()
            .or_else(|| event.payload["session_id"].as_str());
        if event_session.is_some_and(|session_id| session_id != self.state.session.session_id) {
            return;
        }
        if event.payload["sessionEpoch"]
            .as_u64()
            .is_some_and(|epoch| epoch != self.state.session_epoch)
        {
            return;
        }
        match event.kind.as_str() {
            "agent_start" => {
                self.state.run_state = RunState::Running;
                self.state.session.is_streaming = true;
                if !matches!(self.pi_turn, PiTurnState::Active { .. }) {
                    self.begin_pi_turn_timing();
                }
            }
            "agent_end" => {
                // INFO: A low-level agent run may be followed by retry, compaction
                // continuation, or queued messages. The session stays Running and
                // the turn separator is emitted by `agent_settled` (Pi FIFO order).
                if let Some(approval) = self.state.approval.take()
                    && let Some(tool) = self.find_tool_mut(Some(&approval.tool_call_id))
                    && tool.status == ToolStatus::WaitingApproval
                {
                    tool.status = ToolStatus::Denied;
                }
                if self
                    .state
                    .question
                    .as_ref()
                    .is_some_and(|question| question.replying)
                {
                    self.state.question = None;
                }
            }
            "agent_settled" => {
                let pi_turn = std::mem::replace(&mut self.pi_turn, PiTurnState::Inactive);
                match pi_turn {
                    PiTurnState::Active {
                        turn_id,
                        started_at,
                        started,
                    } => {
                        let duration_ms =
                            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                        self.push_turn_separator(turn_id, started_at, duration_ms, false);
                    }
                    PiTurnState::AttachedUnknown => {
                        // Attached mid-run: the start time is unknown, so this is an
                        // estimated <1s boundary rather than a fabricated duration.
                        let wall_clock_ms = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis();
                        let turn_id = format!("pi-agent-{}", self.next_pi_turn_id);
                        self.next_pi_turn_id = self.next_pi_turn_id.saturating_add(1);
                        self.push_turn_separator(
                            turn_id,
                            format!("unix-ms:{wall_clock_ms}"),
                            0,
                            true,
                        );
                    }
                    PiTurnState::Inactive => {
                        // Stale or duplicate settled without a tracked run: ignore.
                        return;
                    }
                }
                self.state.run_state = RunState::Idle;
                self.state.session.is_streaming = false;
            }
            "queue_update" => {
                let steering = event.payload["steering"].as_array().map_or(0, Vec::len);
                let follow_up = event.payload["followUp"].as_array().map_or(0, Vec::len);
                self.state.session.pending_message_count =
                    (steering.saturating_add(follow_up)) as u64;
            }
            "message_start" => {
                if event.payload["message"]["role"].as_str() == Some("assistant") {
                    self.ensure_assistant();
                }
            }
            "message_update" => self.update_message(event.payload),
            "message_end" => {
                if let Some(message) = self.last_assistant_mut() {
                    message.complete = true;
                }
            }
            "tool_execution_start" => {
                let id = string_field(&event.payload, "toolCallId")
                    .unwrap_or_else(|| format!("tool-{}", self.state.transcript.len()));
                let name = string_field(&event.payload, "toolName")
                    .unwrap_or_else(|| "unknown".to_owned());
                self.state
                    .transcript
                    .push(TranscriptItem::Tool(ToolExecution {
                        id,
                        name,
                        args: event.payload["args"].clone(),
                        output: String::new(),
                        diff: None,
                        status: ToolStatus::Running,
                    }));
            }
            "tool_execution_update" => {
                let id = string_field(&event.payload, "toolCallId");
                let output = tool_result_text(&event.payload["partialResult"]);
                if let Some(tool) = self.find_tool_mut(id.as_deref())
                    && let Some(output) = output
                {
                    tool.output = output;
                }
            }
            "tool_execution_end" => {
                let id = string_field(&event.payload, "toolCallId");
                let failed = event.payload["isError"].as_bool().unwrap_or(false);
                let output = tool_result_text(&event.payload["result"]);
                if let Some(tool) = self.find_tool_mut(id.as_deref()) {
                    if let Some(output) = output {
                        tool.output = output;
                    }
                    tool.diff = (!failed)
                        .then(|| parse_tool_diff(&tool.args, &event.payload["result"]["details"]))
                        .flatten();
                    tool.status = if tool.status == ToolStatus::Denied {
                        ToolStatus::Denied
                    } else if failed {
                        ToolStatus::Failed
                    } else {
                        ToolStatus::Succeeded
                    };
                }
                if self
                    .state
                    .approval
                    .as_ref()
                    .is_some_and(|approval| id.as_deref() == Some(approval.tool_call_id.as_str()))
                {
                    self.state.approval = None;
                }
            }
            "compaction_start" => {
                self.state.run_state = RunState::Compacting;
                self.state.session.is_compacting = true;
                self.state.compact_lifecycle_finished = false;
            }
            "compaction_end" => {
                self.state.session.is_compacting = false;
                self.state.compact_lifecycle_finished = true;
                let reason =
                    string_field(&event.payload, "reason").unwrap_or_else(|| "unknown".to_owned());
                let aborted = event.payload["aborted"].as_bool().unwrap_or(false);
                if aborted {
                    self.state.run_state = if self.state.session.is_streaming {
                        RunState::Running
                    } else {
                        RunState::Idle
                    };
                    self.state.transcript.push(TranscriptItem::Error(format!(
                        "{} compaction was aborted.",
                        compaction_reason_label(&reason)
                    )));
                } else if event.payload["result"].is_null() {
                    let error = string_field(&event.payload, "errorMessage").unwrap_or_else(|| {
                        format!("{} compaction failed.", compaction_reason_label(&reason))
                    });
                    self.set_error(error);
                } else {
                    match parse_compaction_record(&event.payload) {
                        Ok(record) => {
                            let key = record.deduplication_key();
                            if self.state.seen_compactions.insert(key) {
                                self.state
                                    .transcript
                                    .push(TranscriptItem::Compaction(record));
                            }
                            self.state.context.usage_state = ContextUsageState::Recalculating;
                            self.state.context.actual_tokens = None;
                            self.state.context.actual_percent = None;
                            self.state.run_state = if self.state.session.is_streaming {
                                RunState::Running
                            } else {
                                RunState::Idle
                            };
                        }
                        Err(error) => self.set_error(error),
                    }
                }
            }
            "error" => {
                let message = event.payload["error"]["message"]
                    .as_str()
                    .or_else(|| event.payload["error"].as_str())
                    .unwrap_or("Pi stream error")
                    .to_owned();
                self.set_pi_error(message);
            }
            _ => {}
        }
    }

    pub(super) fn update_message(&mut self, payload: serde_json::Value) {
        let update = &payload["assistantMessageEvent"];
        let Some(kind) = update["type"].as_str() else {
            return;
        };

        match kind {
            "text_delta" => {
                if let Some(delta) = update["delta"].as_str() {
                    let message = self.ensure_assistant();
                    message.text.push_str(delta);
                    message.text_revision = message.text_revision.saturating_add(1);
                }
            }
            "thinking_delta" => {
                if let Some(delta) = update["delta"].as_str() {
                    let message = self.ensure_assistant();
                    message.thinking.push_str(delta);
                    message.thinking_revision = message.thinking_revision.saturating_add(1);
                }
            }
            "error" => {
                let message = update["error"]["message"]
                    .as_str()
                    .or_else(|| update["error"].as_str())
                    .unwrap_or("Pi message error")
                    .to_owned();
                self.set_pi_error(message);
            }
            _ => {}
        }
    }

    pub(super) fn ensure_assistant(&mut self) -> &mut AssistantMessage {
        let needs_new = !matches!(
            self.state.transcript.last(),
            Some(TranscriptItem::Assistant(message)) if !message.complete
        );
        if needs_new {
            let id = self.state.next_assistant_message_id;
            self.state.next_assistant_message_id =
                self.state.next_assistant_message_id.saturating_add(1);
            self.state
                .transcript
                .push(TranscriptItem::Assistant(AssistantMessage {
                    id,
                    session_epoch: self.state.session_epoch,
                    ..AssistantMessage::default()
                }));
        }
        self.last_assistant_mut()
            .expect("assistant item was just inserted")
    }

    pub(super) fn last_assistant_mut(&mut self) -> Option<&mut AssistantMessage> {
        self.state
            .transcript
            .iter_mut()
            .rev()
            .find_map(|item| match item {
                TranscriptItem::Assistant(message) => Some(message),
                _ => None,
            })
    }

    pub(super) fn find_tool_mut(&mut self, id: Option<&str>) -> Option<&mut ToolExecution> {
        self.state
            .transcript
            .iter_mut()
            .rev()
            .find_map(|item| match item {
                TranscriptItem::Tool(tool)
                    if id.map_or(
                        matches!(
                            tool.status,
                            ToolStatus::WaitingApproval | ToolStatus::Running
                        ),
                        |id| tool.id == id,
                    ) =>
                {
                    Some(tool)
                }
                _ => None,
            })
    }

    pub(super) fn fail_pending_user(&mut self) {
        if let Some(message) = self
            .state
            .transcript
            .iter_mut()
            .rev()
            .find_map(|item| match item {
                TranscriptItem::User(message) if message.status == UserMessageStatus::Pending => {
                    Some(message)
                }
                _ => None,
            })
        {
            message.status = UserMessageStatus::Failed;
        }
    }
}
