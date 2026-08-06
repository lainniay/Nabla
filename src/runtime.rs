//! Execution boundary for effects emitted by the application reducer.

use std::{
    future::Future,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tokio::sync::mpsc;

use crate::{
    app::{AppEffect, AppEvent},
    browser,
    event::CommandEvent,
    file_references::{FileReferenceService, PromptDelivery},
    host::HostClient,
    pi_process::PiClient,
};

#[derive(Clone)]
pub struct EffectDispatcher {
    client: PiClient,
    host: HostClient,
    events: mpsc::Sender<AppEvent>,
    file_references: FileReferenceService,
    latest_file_search_generation: Arc<AtomicU64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchOutcome {
    Continue,
    Quit,
    ExitWithError(String),
}

impl EffectDispatcher {
    pub fn new(
        client: PiClient,
        host: HostClient,
        events: mpsc::Sender<AppEvent>,
        workspace: PathBuf,
    ) -> Result<Self, String> {
        Ok(Self {
            client,
            host,
            events,
            file_references: FileReferenceService::new(workspace)?,
            latest_file_search_generation: Arc::new(AtomicU64::new(0)),
        })
    }

    pub fn dispatch(&self, effect: AppEffect) -> DispatchOutcome {
        match effect {
            AppEffect::Prompt(message) => {
                let client = self.client.clone();
                self.spawn(async move {
                    CommandEvent::PromptFinished(
                        client
                            .prompt(message, None)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::Steer(message) => {
                let client = self.client.clone();
                self.spawn(async move {
                    CommandEvent::PromptFinished(
                        client
                            .steer(message, None)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::FollowUp(message) => {
                let client = self.client.clone();
                self.spawn(async move {
                    CommandEvent::PromptFinished(
                        client
                            .follow_up(message, None)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::SearchFiles { query, generation } => {
                let service = self.file_references.clone();
                let latest_generation = self.latest_file_search_generation.clone();
                let events = self.events.clone();
                latest_generation.store(generation, Ordering::Release);
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(75)).await;
                    if latest_generation.load(Ordering::Acquire) != generation {
                        return;
                    }
                    let result = tokio::task::spawn_blocking(move || service.search(&query))
                        .await
                        .map_err(|error| format!("File search task failed: {error}"))
                        .and_then(|result| result);
                    if latest_generation.load(Ordering::Acquire) != generation {
                        return;
                    }
                    let _ = events
                        .send(AppEvent::Command(CommandEvent::FileSearchFinished {
                            generation,
                            result,
                        }))
                        .await;
                });
            }
            AppEffect::PrepareReferences { message, delivery } => {
                let service = self.file_references.clone();
                self.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || service.prepare(message))
                        .await
                        .map_err(|error| format!("Reference preparation task failed: {error}"))
                        .and_then(|result| result);
                    CommandEvent::ReferencesPrepared { delivery, result }
                });
            }
            AppEffect::DeliverPrepared { prompt, delivery } => {
                let client = self.client.clone();
                self.spawn(async move {
                    let images = Some(prompt.images);
                    let result = match delivery {
                        PromptDelivery::Prompt => client.prompt(prompt.message, images).await,
                        PromptDelivery::Steer => client.steer(prompt.message, images).await,
                        PromptDelivery::FollowUp => client.follow_up(prompt.message, images).await,
                    };
                    CommandEvent::PromptFinished(result.map_err(|error| error.to_string()))
                });
            }
            AppEffect::Abort => {
                let client = self.client.clone();
                self.spawn(async move {
                    CommandEvent::AbortFinished(
                        client.abort().await.map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::ClearQueue => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::QueueCleared(
                        host.clear_queue()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::AbortAndClearQueue => {
                let client = self.client.clone();
                let host = self.host.clone();
                self.spawn(async move {
                    let queue = host.clear_queue().await;
                    let abort = client.abort().await;
                    let result = match (queue, abort) {
                        (Ok(queue), Ok(())) => Ok(Box::new(queue)),
                        (_, Err(error)) | (Err(error), _) => Err(error.to_string()),
                    };
                    CommandEvent::AbortAndQueueCleared(result)
                });
            }
            AppEffect::Compact(instructions) => {
                let client = self.client.clone();
                self.spawn(async move {
                    CommandEvent::CompactFinished(
                        client
                            .compact(instructions)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::GetContextState => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::ContextStateFinished(
                        host.get_context_state()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::GetResources => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::ResourcesFinished(
                        host.get_resources()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::ReloadResources => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::ResourceReloadFinished(
                        host.reload_resources()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::SetWorkspaceTrust(trusted) => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::WorkspaceTrustFinished(
                        host.set_workspace_trust(trusted)
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::GetApprovalRules => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::ApprovalRulesFinished(
                        host.get_approval_rules()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::RevokeApprovalRule(rule_id) => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::ApprovalRuleRevoked(
                        host.revoke_approval_rule(rule_id)
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::ClearApprovalRules => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::ApprovalRulesCleared(
                        host.clear_approval_rules()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::ListModels => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::ModelsFinished(
                        host.list_models()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::SetModel { provider, model_id } => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::ModelSetFinished(
                        host.set_model(provider, model_id)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::SetThinking(level) => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::ThinkingSetFinished(
                        host.set_thinking(level)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::GetAgents => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::AgentsFinished(
                        host.get_agents()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::ReloadAgents => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::AgentsReloaded(
                        host.reload_agents()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::StartSubagent { profile, task } => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::SubagentStarted(
                        host.start_subagent(profile, task)
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::CancelSubagent(agent_id) => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::SubagentCancelled(
                        host.cancel_subagent(agent_id)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::IntegrateSubagent { agent_id, action } => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::SubagentIntegrated(
                        host.integrate_subagent(agent_id, action)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::OpenSessionBrowser => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::SessionBrowserOpened(
                        host.open_session_browser()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::QuerySessionBrowser {
                browser_id,
                scope,
                query,
                sort_mode,
                named_only,
                offset,
                generation,
            } => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::SessionBrowserQueryFinished {
                        generation,
                        result: host
                            .query_session_browser(
                                browser_id, scope, query, sort_mode, named_only, offset,
                            )
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    }
                });
            }
            AppEffect::CloseSessionBrowser { browser_id } => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::SessionBrowserClosed(
                        host.close_session_browser(browser_id)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::NewSession => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::NewSessionFinished(
                        host.new_session()
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::ResumeSession {
                session_path,
                cwd_override,
            } => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::ResumeSessionFinished(
                        host.resume_session(session_path, cwd_override)
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::GetTreeState {
                filter_mode,
                query,
                folded_entry_ids,
                generation,
            } => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::TreeStateFinished {
                        generation,
                        result: host
                            .get_tree_state(filter_mode, query, folded_entry_ids)
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    }
                });
            }
            AppEffect::SetTreeLabel { entry_id, label } => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::TreeLabelFinished(
                        host.set_tree_label(entry_id, label)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::CopyTreeEntry { entry_id } => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::TreeCopyFinished(
                        host.copy_tree_entry(entry_id)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::NavigateTree {
                entry_id,
                summarize,
                custom_instructions,
            } => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::TreeNavigateFinished(
                        host.navigate_tree(entry_id, summarize, custom_instructions)
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::AbortTreeNavigation => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::TreeAbortFinished(
                        host.abort_tree_navigation()
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::AuthList => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::AuthProvidersFinished(
                        host.list_providers()
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::AuthLogin {
                flow_id,
                provider_id,
                auth_type,
            } => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::AuthLoginFinished(
                        host.login(flow_id, provider_id, auth_type)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::AuthReply {
                flow_id,
                prompt_id,
                value,
            } => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::AuthReplyFinished(
                        host.reply(flow_id, prompt_id, value.into_inner())
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::AuthCancel { flow_id } => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::AuthCancelFinished(
                        host.cancel(flow_id)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::OpenUrl(url) => {
                self.spawn(
                    async move { CommandEvent::OpenUrlFinished(browser::open_url(&url).await) },
                );
            }
            AppEffect::SetPlanMode(active) => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::SetPlanModeFinished {
                        requested: active,
                        result: host
                            .set_plan_mode(active)
                            .await
                            .map_err(|error| error.to_string()),
                    }
                });
            }
            AppEffect::ReplyApproval {
                approval_id,
                decision,
            } => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::ApprovalReplyFinished {
                        approval_id: approval_id.clone(),
                        decision,
                        result: host
                            .reply_approval(approval_id, decision)
                            .await
                            .map_err(|error| error.to_string()),
                    }
                });
            }
            AppEffect::ReplyQuestions {
                request_id,
                answers,
            } => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::QuestionReplyFinished(
                        host.reply_questions(request_id, answers)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            AppEffect::ExecutePlan(context) => {
                let host = self.host.clone();
                self.spawn(async move {
                    CommandEvent::PlanExecutionFinished {
                        context,
                        result: host
                            .execute_plan(context)
                            .await
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    }
                });
            }
            AppEffect::Quit => return DispatchOutcome::Quit,
            AppEffect::ExitWithError(error) => return DispatchOutcome::ExitWithError(error),
        }
        DispatchOutcome::Continue
    }

    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = CommandEvent> + Send + 'static,
    {
        let events = self.events.clone();
        tokio::spawn(async move {
            let _ = events.send(AppEvent::Command(future.await)).await;
        });
    }
}
