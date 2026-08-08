use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use super::request::UnixSocketRules;
use super::request::{NetworkProfile, SandboxExecRequest};

#[derive(Debug, Clone)]
pub struct CompiledProfile {
    pub cwd: PathBuf,
    pub read_write: Vec<PathBuf>,
    pub deny_read: Vec<PathBuf>,
    pub deny_write: Vec<PathBuf>,
    pub network: NetworkProfile,
    pub unix_sockets: UnixSocketRules,
}

pub fn compile(request: &SandboxExecRequest) -> Result<CompiledProfile, String> {
    let cwd = canonicalize_path(&request.cwd)?;
    let mut read_write = BTreeSet::new();
    let mut deny_read = BTreeSet::new();
    let mut deny_write = BTreeSet::new();

    for path in &request.profile.filesystem.read_write {
        read_write.insert(canonicalize_path(path)?);
    }
    for path in &request.profile.filesystem.read_only {
        deny_write.insert(canonicalize_path(path)?);
    }
    for path in &request.profile.filesystem.deny_read {
        deny_read.insert(canonicalize_path(path)?);
    }
    for path in &request.profile.filesystem.deny_write {
        deny_write.insert(canonicalize_path(path)?);
    }
    for path in &request.profile.protected_paths {
        let canonical = canonicalize_path(path)?;
        deny_read.insert(canonical.clone());
        deny_write.insert(canonical);
    }

    read_write.insert(cwd.clone());
    let read_write: Vec<PathBuf> = read_write
        .into_iter()
        .filter(|path| !deny_write.contains(path))
        .collect();
    let unix_sockets = UnixSocketRules {
        allow: request
            .profile
            .unix_sockets
            .allow
            .iter()
            .map(|path| canonicalize_path(path))
            .collect::<Result<Vec<_>, _>>()?,
        deny: request
            .profile
            .unix_sockets
            .deny
            .iter()
            .map(|path| canonicalize_path(path))
            .collect::<Result<Vec<_>, _>>()?,
    };

    Ok(CompiledProfile {
        cwd,
        read_write,
        deny_read: deny_read.into_iter().collect(),
        deny_write: deny_write.into_iter().collect(),
        network: request.profile.network,
        unix_sockets,
    })
}

fn canonicalize_path(path: &Path) -> Result<PathBuf, String> {
    if let Ok(canonical) = path.canonicalize() {
        return Ok(canonical);
    }
    let mut current = path.to_path_buf();
    let mut suffix = PathBuf::new();
    loop {
        if let Ok(canonical) = current.canonicalize() {
            return Ok(canonical.join(suffix).components().collect());
        }
        let Some(name) = current.file_name() else {
            break;
        };
        suffix = PathBuf::from(name).join(suffix);
        if !current.pop() {
            break;
        }
    }
    Err(format!("cannot canonicalize path: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::SandboxMode;
    use crate::sandbox::request::{FilesystemProfile, SandboxExecRequest, SandboxProfile};
    use std::collections::BTreeMap;

    fn request() -> SandboxExecRequest {
        SandboxExecRequest {
            version: 1,
            mode: SandboxMode::Enforced,
            cwd: "/workspace".into(),
            command: "true".into(),
            timeout_ms: None,
            profile: SandboxProfile {
                filesystem: FilesystemProfile {
                    read_only: vec!["/etc".into()],
                    read_write: vec!["/tmp/nabla".into()],
                    deny_read: vec!["/home/user/.ssh".into()],
                    deny_write: vec!["/workspace/.env".into()],
                },
                network: NetworkProfile::Deny,
                protected_paths: vec!["/home/user/.aws".into()],
                unix_sockets: Default::default(),
            },
            environment: BTreeMap::new(),
        }
    }

    #[test]
    fn protected_paths_cannot_be_written_or_read() {
        let compiled = compile(&request()).unwrap();
        assert!(compiled.deny_read.iter().any(|path| path.ends_with(".aws")));
        assert!(
            compiled
                .deny_write
                .iter()
                .any(|path| path.ends_with(".aws"))
        );
    }

    #[test]
    fn read_only_paths_are_not_writable() {
        let compiled = compile(&request()).unwrap();
        assert!(compiled.deny_write.iter().any(|path| path.ends_with("etc")));
    }
}
