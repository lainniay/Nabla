use super::*;
use crate::state::{SessionScope, SessionSortMode, TreeFilterMode};
use std::path::PathBuf;
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
    assert!(local.control_socket.starts_with(std::env::temp_dir()));
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
            "delegate_task",
            "todo_write"
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
            "delegate_task",
            "todo_write"
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
