use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use ignore::WalkBuilder;

use crate::file_references::matcher::{match_score, path_depth, slash_path};
use crate::file_references::model::{FileCandidate, FileIndexCache, MAX_INDEX_FILES};

#[derive(Clone)]
pub struct FileReferenceService {
    pub(crate) root: PathBuf,
    pub(crate) cache: FileIndexCache,
}

impl FileReferenceService {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|error| format!("Unable to resolve workspace: {error}"))?;
        Ok(Self {
            root,
            cache: Arc::new(Mutex::new(None)),
        })
    }

    pub fn search(&self, query: &str) -> Result<Vec<FileCandidate>, String> {
        let files = self.index()?;
        let query = query.to_lowercase();
        let mut matches = files
            .into_iter()
            .filter_map(|candidate| match_score(&candidate, &query).map(|score| (score, candidate)))
            .collect::<Vec<_>>();
        matches.sort_by(|(left_score, left), (right_score, right)| {
            left_score
                .cmp(right_score)
                .then_with(|| path_depth(&left.path).cmp(&path_depth(&right.path)))
                .then_with(|| left.path.to_lowercase().cmp(&right.path.to_lowercase()))
        });
        Ok(matches
            .into_iter()
            .map(|(_, candidate)| candidate)
            .take(50)
            .collect())
    }
}
impl FileReferenceService {
    fn index(&self) -> Result<Vec<FileCandidate>, String> {
        if let Some((created, files)) = self
            .cache
            .lock()
            .map_err(|_| "File index cache is unavailable".to_owned())?
            .as_ref()
            && created.elapsed() < Duration::from_secs(2)
        {
            return Ok(files.clone());
        }
        let mut files = Vec::new();
        let walker = WalkBuilder::new(&self.root)
            .hidden(false)
            .require_git(false)
            .follow_links(false)
            .filter_entry(|entry| entry.file_name() != ".git")
            .build();
        for entry in walker {
            let entry = entry.map_err(|error| format!("Unable to index workspace: {error}"))?;
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let metadata = entry.metadata().map_err(|error| {
                format!("Unable to inspect {}: {error}", entry.path().display())
            })?;
            let relative = entry
                .path()
                .strip_prefix(&self.root)
                .map_err(|_| "Indexed path escaped workspace".to_owned())?;
            let path = slash_path(relative);
            let basename = entry.file_name().to_string_lossy().into_owned();
            let parent = relative
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .map(slash_path)
                .unwrap_or_default();
            files.push(FileCandidate {
                path,
                basename,
                parent,
                size: metadata.len(),
            });
            if files.len() >= MAX_INDEX_FILES {
                break;
            }
        }
        *self
            .cache
            .lock()
            .map_err(|_| "File index cache is unavailable".to_owned())? =
            Some((Instant::now(), files.clone()));
        Ok(files)
    }
}
