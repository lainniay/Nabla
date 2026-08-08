use std::collections::BTreeSet;
use std::path::Path;

use super::{
    profile::CompiledProfile,
    request::{NetworkProfile, SandboxExecRequest},
};

pub fn command(request: &SandboxExecRequest, compiled: &CompiledProfile) -> (String, Vec<String>) {
    let mut profile = String::from("(version 1)\n(allow default)\n");
    if compiled.network == NetworkProfile::Deny {
        profile.push_str("(deny network*)\n");
        profile
            .push_str("; allow local unix sockets beneath writable roots and explicit allowlist\n");
        profile.push_str("(allow system-socket (socket-domain AF_UNIX))\n");
        let mut socket_roots = BTreeSet::new();
        socket_roots.extend(compiled.read_write.iter().cloned());
        socket_roots.extend(compiled.unix_sockets.allow.iter().cloned());
        for path in socket_roots {
            profile.push_str(&format!(
                "(allow network-bind (local unix-socket (subpath \"{}\")))\n",
                escape(&path)
            ));
            profile.push_str(&format!(
                "(allow network-outbound (remote unix-socket (subpath \"{}\")))\n",
                escape(&path)
            ));
        }
        for path in &compiled.unix_sockets.deny {
            profile.push_str(&format!(
                "(deny network-bind (local unix-socket (subpath \"{}\")))\n",
                escape(path)
            ));
            profile.push_str(&format!(
                "(deny network-outbound (remote unix-socket (subpath \"{}\")))\n",
                escape(path)
            ));
        }
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
    use crate::sandbox::request::{
        FilesystemProfile, SandboxExecRequest, SandboxProfile, UnixSocketRules,
    };
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
                unix_sockets: UnixSocketRules {
                    allow: vec!["/opt/nabla-allowed.sock".into()],
                    deny: vec!["/workspace/.env.sock".into()],
                },
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
        assert!(profile.contains("(allow system-socket (socket-domain AF_UNIX))"));
        assert!(
            profile.contains("(allow network-bind (local unix-socket (subpath \"/workspace\")))")
        );
        assert!(
            profile
                .contains("(allow network-outbound (remote unix-socket (subpath \"/workspace\")))")
        );
        assert!(profile.contains(
            "(allow network-bind (local unix-socket (subpath \"/opt/nabla-allowed.sock\")))"
        ));
        assert!(profile.contains(
            "(allow network-outbound (remote unix-socket (subpath \"/opt/nabla-allowed.sock\")))"
        ));
        assert!(profile.contains(
            "(deny network-bind (local unix-socket (subpath \"/workspace/.env.sock\")))"
        ));
        assert!(profile.contains(
            "(deny network-outbound (remote unix-socket (subpath \"/workspace/.env.sock\")))"
        ));
        assert!(profile.contains("(deny file-read* (subpath \"/Users/test/.ssh\"))"));
        assert!(profile.contains("(deny file-write* (subpath \"/workspace/.env\"))"));
    }

    #[test]
    fn network_allow_omits_unix_socket_rules() {
        let mut request = request();
        request.profile.network = NetworkProfile::Allow;
        let compiled: CompiledProfile = compile(&request).unwrap();
        let (_, args) = command(&request, &compiled);
        let profile = &args[1];
        assert!(!profile.contains("(deny network*)"));
        assert!(!profile.contains("unix-socket"));
    }
}
