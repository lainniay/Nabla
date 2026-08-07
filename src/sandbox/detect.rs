use std::process::Command;

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxCapability {
    pub mode: String,
    pub backend: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub supports_filesystem_isolation: bool,
    pub supports_network_isolation: bool,
}

pub fn detect() -> SandboxCapability {
    if std::env::var("NABLA_SANDBOX_DISABLED").as_deref() == Ok("1") {
        return SandboxCapability {
            mode: "disabled".to_owned(),
            backend: "none".to_owned(),
            reason: Some("explicitly disabled via NABLA_SANDBOX_DISABLED".to_owned()),
            supports_filesystem_isolation: false,
            supports_network_isolation: false,
        };
    }
    #[cfg(target_os = "linux")]
    {
        match probe_bwrap() {
            Ok(()) => enforced("bubblewrap", None),
            Err(reason) => degraded(reason),
        }
    }
    #[cfg(target_os = "macos")]
    {
        match probe_seatbelt() {
            Ok(()) => enforced("seatbelt", None),
            Err(reason) => degraded(reason),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        degraded("no native sandbox backend on this platform".to_owned())
    }
}

fn enforced(backend: &str, reason: Option<String>) -> SandboxCapability {
    SandboxCapability {
        mode: "enforced".to_owned(),
        backend: backend.to_owned(),
        reason,
        supports_filesystem_isolation: true,
        supports_network_isolation: true,
    }
}

fn degraded(reason: String) -> SandboxCapability {
    SandboxCapability {
        mode: "degraded".to_owned(),
        backend: "none".to_owned(),
        reason: Some(reason),
        supports_filesystem_isolation: false,
        supports_network_isolation: false,
    }
}

#[cfg(target_os = "linux")]
fn probe_bwrap() -> Result<(), String> {
    let output = Command::new("bwrap")
        .args([
            "--ro-bind",
            "/",
            "/",
            "--dev",
            "/dev",
            "--unshare-all",
            "--",
            "/bin/true",
        ])
        .output()
        .map_err(|error| format!("bwrap is not available: {error}"))?;
    if !output.status.success() {
        return Err(format!("bwrap probe failed: {}", output.status));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn probe_seatbelt() -> Result<(), String> {
    let output = Command::new("sandbox-exec")
        .args(["-p", "(version 1)\n(allow default)", "/usr/bin/true"])
        .output()
        .map_err(|error| format!("sandbox-exec is not available: {error}"))?;
    if !output.status.success() {
        return Err(format!("sandbox-exec probe failed: {}", output.status));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_env_is_authoritative() {
        unsafe {
            std::env::set_var("NABLA_SANDBOX_DISABLED", "1");
        }
        let capability = detect();
        unsafe {
            std::env::remove_var("NABLA_SANDBOX_DISABLED");
        }
        assert_eq!(capability.mode, "disabled");
        assert_eq!(capability.backend, "none");
    }
}
