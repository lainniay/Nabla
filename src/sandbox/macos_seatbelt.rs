use std::path::Path;

use super::{
    profile::CompiledProfile,
    request::{NetworkProfile, SandboxExecRequest},
};

pub fn command(request: &SandboxExecRequest, compiled: &CompiledProfile) -> (String, Vec<String>) {
    let mut profile = String::from("(version 1)\n(allow default)\n");
    if compiled.network == NetworkProfile::Deny {
        profile.push_str("(deny network*)\n");
    }
    profile.push_str("(deny file-write*)\n");
    profile.push_str("(allow file-write* (literal \"/dev/null\"))\n");
    for path in &compiled.read_write {
        profile.push_str(&format!(
            "(allow file-write* (subpath \"{}\"))\n",
            escape(path)
        ));
    }
    for path in &compiled.deny_read {
        profile.push_str(&format!(
            "(deny file-read* (subpath \"{}\"))\n",
            escape(path)
        ));
    }
    for path in &compiled.deny_write {
        profile.push_str(&format!(
            "(deny file-write* (subpath \"{}\"))\n",
            escape(path)
        ));
    }

    (
        "sandbox-exec".to_owned(),
        vec![
            "-p".to_owned(),
            profile,
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            request.command.clone(),
        ],
    )
}

fn escape(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::SandboxMode;
    use crate::sandbox::profile::{CompiledProfile, compile};
    use crate::sandbox::request::{FilesystemProfile, SandboxExecRequest, SandboxProfile};
    use std::collections::BTreeMap;

    fn request() -> SandboxExecRequest {
        SandboxExecRequest {
            version: 1,
            mode: SandboxMode::Enforced,
            cwd: "/workspace".into(),
            command: "echo hi".into(),
            timeout_ms: None,
            profile: SandboxProfile {
                filesystem: FilesystemProfile {
                    read_only: vec![],
                    read_write: vec!["/workspace".into()],
                    deny_read: vec!["/Users/test/.ssh".into()],
                    deny_write: vec!["/workspace/.env".into()],
                },
                network: NetworkProfile::Deny,
                protected_paths: vec![],
            },
            environment: BTreeMap::new(),
        }
    }

    #[test]
    fn profile_denies_network_and_protected_paths() {
        let compiled: CompiledProfile = compile(&request()).unwrap();
        let (program, args) = command(&request(), &compiled);
        assert_eq!(program, "sandbox-exec");
        let profile = &args[1];
        assert!(profile.contains("(deny network*)"));
        assert!(profile.contains("(deny file-write*)"));
        assert!(profile.contains("(allow file-write* (literal \"/dev/null\"))"));
        assert!(profile.contains("(allow file-write* (subpath \"/workspace\"))"));
        assert!(profile.contains("(deny file-read* (subpath \"/Users/test/.ssh\"))"));
        assert!(profile.contains("(deny file-write* (subpath \"/workspace/.env\"))"));
    }
}
