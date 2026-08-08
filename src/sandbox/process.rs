use std::{
    collections::BTreeMap,
    io::{Read, Write},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use super::{linux_bwrap, macos_seatbelt, profile::CompiledProfile, request::SandboxExecRequest};

const DANGEROUS_ENV: &[&str] = &[
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "BASH_ENV",
    "ENV",
    "PROMPT_COMMAND",
    "GIT_SSH_COMMAND",
    "SSH_AUTH_SOCK",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "NPM_TOKEN",
];

pub fn run(request: &SandboxExecRequest, compiled: &CompiledProfile) -> Result<i32, String> {
    let (program, args) = if cfg!(target_os = "linux") {
        linux_bwrap::command(request, compiled)
    } else if cfg!(target_os = "macos") {
        macos_seatbelt::command(request, compiled)
    } else {
        return Err("no native sandbox backend on this platform".to_owned());
    };

    let mut command = Command::new(&program);
    command
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(filtered_environment(request));
    #[cfg(unix)]
    command.process_group(0);

    let child = command
        .spawn()
        .map_err(|error| format!("failed to spawn {program}: {error}"))?;
    wait_for_child(child, request.timeout_ms)
}

pub fn run_plain(request: &SandboxExecRequest) -> Result<i32, String> {
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(&request.command)
        .current_dir(&request.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(filtered_environment(request));
    #[cfg(unix)]
    command.process_group(0);

    let child = command
        .spawn()
        .map_err(|error| format!("failed to spawn /bin/sh: {error}"))?;
    wait_for_child(child, request.timeout_ms)
}

fn wait_for_child(mut child: Child, timeout_ms: Option<u64>) -> Result<i32, String> {
    let mut stdout = child.stdout.take().ok_or("sandbox stdout unavailable")?;
    let mut stderr = child.stderr.take().ok_or("sandbox stderr unavailable")?;
    let stdout_thread = thread::spawn(move || {
        pump(&mut stdout, &mut std::io::stdout().lock());
    });
    let stderr_thread = thread::spawn(move || {
        pump(&mut stderr, &mut std::io::stderr().lock());
    });

    let deadline = timeout_ms.map(|millis| Instant::now() + Duration::from_millis(millis));
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                return Err(format!("sandbox wait failed: {error}"));
            }
        }
        if let Some(deadline) = deadline
            && Instant::now() >= deadline
        {
            kill_process_group(child.id());
            let _ = child.wait();
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            return Err("sandbox command timed out".to_owned());
        }
        thread::sleep(Duration::from_millis(20));
    };
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();
    Ok(status.code().unwrap_or(1))
}

pub(crate) fn filtered_environment(request: &SandboxExecRequest) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    for (key, value) in std::env::vars() {
        if !DANGEROUS_ENV.contains(&key.as_str()) {
            environment.insert(key, value);
        }
    }
    for (key, value) in &request.environment {
        environment.insert(key.clone(), value.clone());
    }
    environment
}

fn pump<R: Read, W: Write>(reader: &mut R, writer: &mut W) {
    let mut buffer = [0u8; 8192];
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => size,
            Err(_) => break,
        };
        if writer.write_all(&buffer[..read]).is_err() {
            break;
        }
    }
}

#[cfg(unix)]
fn kill_process_group(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", &format!("-{pid}")])
        .status();
}

#[cfg(not(unix))]
fn kill_process_group(_pid: u32) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::SandboxMode;
    use crate::sandbox::request::{FilesystemProfile, SandboxExecRequest, SandboxProfile};

    #[test]
    fn filters_dangerous_environment_variables() {
        unsafe {
            std::env::set_var("LD_PRELOAD", "/tmp/evil.so");
        }
        let request = SandboxExecRequest {
            version: 1,
            mode: SandboxMode::Enforced,
            cwd: "/workspace".into(),
            command: "true".into(),
            timeout_ms: None,
            profile: SandboxProfile {
                filesystem: FilesystemProfile::default(),
                network: crate::sandbox::NetworkProfile::Deny,
                protected_paths: vec![],
                unix_sockets: Default::default(),
            },
            environment: BTreeMap::new(),
        };
        let environment = filtered_environment(&request);
        unsafe {
            std::env::remove_var("LD_PRELOAD");
        }
        assert!(!environment.contains_key("LD_PRELOAD"));
    }

    #[test]
    fn plain_execution_runs_the_command_and_returns_its_exit_code() {
        let request = SandboxExecRequest {
            version: 1,
            mode: SandboxMode::Degraded,
            cwd: std::env::current_dir().unwrap(),
            command: "exit 7".into(),
            timeout_ms: None,
            profile: SandboxProfile {
                filesystem: FilesystemProfile::default(),
                network: crate::sandbox::NetworkProfile::Deny,
                protected_paths: vec![],
                unix_sockets: Default::default(),
            },
            environment: BTreeMap::new(),
        };
        assert_eq!(run_plain(&request).unwrap(), 7);
    }
}
