use std::{collections::BTreeMap, io::Read, path::PathBuf};

use serde::Deserialize;

pub const REQUEST_VERSION: u32 = 1;
const MAX_REQUEST_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    #[default]
    Enforced,
    Degraded,
    Disabled,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxExecRequest {
    pub version: u32,
    #[serde(default)]
    pub mode: SandboxMode,
    pub cwd: PathBuf,
    pub command: String,
    pub timeout_ms: Option<u64>,
    pub profile: SandboxProfile,
    pub environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxProfile {
    pub filesystem: FilesystemProfile,
    pub network: NetworkProfile,
    pub protected_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesystemProfile {
    #[serde(default)]
    pub read_only: Vec<PathBuf>,
    #[serde(default)]
    pub read_write: Vec<PathBuf>,
    #[serde(default)]
    pub deny_read: Vec<PathBuf>,
    #[serde(default)]
    pub deny_write: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NetworkProfile {
    Deny,
    Allow,
}

impl SandboxExecRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != REQUEST_VERSION {
            return Err(format!(
                "unsupported sandbox request version: {}",
                self.version
            ));
        }
        if !self.cwd.is_absolute() {
            return Err("sandbox cwd must be absolute".to_owned());
        }
        if self.command.contains('\0') {
            return Err("sandbox command contains NUL".to_owned());
        }
        if self.timeout_ms == Some(0) {
            return Err("sandbox timeout must be positive".to_owned());
        }
        for (key, value) in &self.environment {
            if key.contains('\0') || value.contains('\0') {
                return Err("sandbox environment contains NUL".to_owned());
            }
        }
        for path in self.profile.filesystem.all_paths() {
            if !path.is_absolute() {
                return Err(format!(
                    "sandbox profile path must be absolute: {}",
                    path.display()
                ));
            }
        }
        for path in &self.profile.protected_paths {
            if !path.is_absolute() {
                return Err(format!(
                    "sandbox protected path must be absolute: {}",
                    path.display()
                ));
            }
        }
        Ok(())
    }
}

impl FilesystemProfile {
    fn all_paths(&self) -> impl Iterator<Item = &std::path::Path> {
        self.read_only
            .iter()
            .chain(&self.read_write)
            .chain(&self.deny_read)
            .chain(&self.deny_write)
            .map(std::path::PathBuf::as_path)
    }
}

pub fn read_request() -> Result<SandboxExecRequest, String> {
    let mut buffer = Vec::new();
    let mut limited = std::io::stdin().lock().take(MAX_REQUEST_BYTES + 1);
    limited
        .read_to_end(&mut buffer)
        .map_err(|error| format!("failed to read sandbox request: {error}"))?;
    if buffer.len() as u64 > MAX_REQUEST_BYTES {
        return Err("sandbox request exceeds size limit".to_owned());
    }
    let request: SandboxExecRequest = serde_json::from_slice(&buffer)
        .map_err(|error| format!("invalid sandbox request: {error}"))?;
    request.validate()?;
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_relative_paths_and_nul() {
        let request: SandboxExecRequest = serde_json::from_value(json!({
            "version": 1,
            "cwd": "/workspace",
            "command": "echo hi",
            "profile": {
                "filesystem": {
                    "readWrite": ["relative"],
                    "readOnly": [],
                    "denyRead": [],
                    "denyWrite": []
                },
                "network": "deny",
                "protectedPaths": []
            },
            "environment": {}
        }))
        .unwrap();
        assert!(request.validate().is_err());

        let mut bad = request.clone();
        bad.command = "echo \0hi".to_owned();
        assert!(bad.validate().is_err());
    }

    #[test]
    fn round_trips_camel_case_network() {
        let profile: SandboxProfile = serde_json::from_value(json!({
            "filesystem": {
                "readOnly": [],
                "readWrite": [],
                "denyRead": [],
                "denyWrite": []
            },
            "network": "deny",
            "protectedPaths": []
        }))
        .unwrap();
        assert_eq!(profile.network, NetworkProfile::Deny);
    }

    #[test]
    fn shared_exec_fixtures_parse_and_validate() {
        for (fixture, expected) in [
            (
                include_str!("../../protocol-fixtures/sandbox/exec-enforced.json"),
                SandboxMode::Enforced,
            ),
            (
                include_str!("../../protocol-fixtures/sandbox/exec-degraded.json"),
                SandboxMode::Degraded,
            ),
        ] {
            let request: SandboxExecRequest =
                serde_json::from_str(fixture).expect("fixture must parse");
            request.validate().expect("fixture must validate");
            assert_eq!(request.mode, expected);
        }
    }
}
