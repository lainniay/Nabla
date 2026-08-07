use std::path::Path;

use super::{process::filtered_environment, profile::CompiledProfile, request::SandboxExecRequest};

pub fn command(request: &SandboxExecRequest, compiled: &CompiledProfile) -> (String, Vec<String>) {
    let mut args = vec![
        "bwrap".to_owned(),
        "--unshare-user".to_owned(),
        "--unshare-pid".to_owned(),
        "--unshare-net".to_owned(),
        "--unshare-ipc".to_owned(),
        "--unshare-uts".to_owned(),
        "--ro-bind".to_owned(),
        "/".to_owned(),
        "/".to_owned(),
        "--proc".to_owned(),
        "/proc".to_owned(),
        "--dev".to_owned(),
        "/dev".to_owned(),
        "--tmpfs".to_owned(),
        "/tmp".to_owned(),
    ];

    for path in &compiled.read_write {
        bind("--bind", path, &mut args);
    }
    for path in &compiled.deny_read {
        args.push("--tmpfs".to_owned());
        args.push(path.to_string_lossy().into_owned());
    }
    for path in &compiled.deny_write {
        bind("--ro-bind", path, &mut args);
    }

    args.push("--clearenv".to_owned());
    for (key, value) in filtered_environment(request) {
        args.push("--setenv".to_owned());
        args.push(key);
        args.push(value);
    }
    args.push("--chdir".to_owned());
    args.push(compiled.cwd.to_string_lossy().into_owned());
    args.push("--die-with-parent".to_owned());
    args.push("--".to_owned());
    args.push("/bin/sh".to_owned());
    args.push("-c".to_owned());
    args.push(request.command.clone());

    ("bwrap".to_owned(), args)
}

fn bind(flag: &str, path: &Path, args: &mut Vec<String>) {
    let path = path.to_string_lossy().into_owned();
    args.push(flag.to_owned());
    args.push(path.clone());
    args.push(path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::profile::{CompiledProfile, compile};
    use crate::sandbox::request::{FilesystemProfile, SandboxExecRequest, SandboxProfile};
    use std::collections::BTreeMap;

    fn request() -> SandboxExecRequest {
        SandboxExecRequest {
            version: 1,
            cwd: "/workspace".into(),
            command: "echo hi".into(),
            timeout_ms: None,
            profile: SandboxProfile {
                filesystem: FilesystemProfile {
                    read_only: vec![],
                    read_write: vec!["/workspace".into(), "/tmp/nabla".into()],
                    deny_read: vec!["/home/user/.ssh".into()],
                    deny_write: vec!["/workspace/.env".into()],
                },
                network: crate::sandbox::NetworkProfile::Deny,
                protected_paths: vec![],
            },
            environment: BTreeMap::new(),
        }
    }

    #[test]
    fn builds_mount_order_and_shell_argv() {
        let compiled: CompiledProfile = compile(&request()).unwrap();
        let (program, args) = command(&request(), &compiled);
        assert_eq!(program, "bwrap");
        let joined = args.join(" ");
        assert!(joined.contains("--unshare-net"));
        assert!(joined.contains("--bind /workspace /workspace"));
        assert!(joined.contains("--tmpfs"));
        assert!(joined.contains(".ssh"));
        assert!(joined.contains("--ro-bind /workspace/.env /workspace/.env"));
        assert!(joined.ends_with("/bin/sh -c echo hi"));
    }
}
