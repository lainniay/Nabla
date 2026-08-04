use std::{collections::HashMap, sync::Arc};

use crate::state::{ToolExecution, ToolStatus};

#[derive(Debug, Clone, Default)]
pub struct ToolSnapshotStore {
    latest: HashMap<String, Arc<ToolExecution>>,
    revisions: HashMap<String, u64>,
}

impl ToolSnapshotStore {
    /// Replaces a partial snapshot atomically. Intermediate snapshots may be
    /// skipped; rendering always reads the newest authoritative value.
    pub fn update(&mut self, snapshot: ToolExecution, revision: u64) -> bool {
        let id = snapshot.id.clone();
        if self
            .revisions
            .get(&id)
            .is_some_and(|current| *current > revision)
        {
            return false;
        }
        self.revisions.insert(id.clone(), revision);
        self.latest.insert(id, Arc::new(snapshot));
        true
    }

    pub fn get(&self, id: &str) -> Option<&Arc<ToolExecution>> {
        self.latest.get(id)
    }

    pub fn finish(&mut self, snapshot: ToolExecution, revision: u64) -> bool {
        debug_assert!(matches!(
            snapshot.status,
            ToolStatus::Succeeded | ToolStatus::Failed | ToolStatus::Denied
        ));
        self.update(snapshot, revision)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn snapshot(output: &str, status: ToolStatus) -> ToolExecution {
        ToolExecution {
            id: "call".to_owned(),
            name: "read".to_owned(),
            args: json!({"path": "file"}),
            output: output.to_owned(),
            diff: None,
            status,
        }
    }

    #[test]
    fn coalescing_does_not_depend_on_receiving_every_partial_event() {
        let mut store = ToolSnapshotStore::default();
        assert!(store.update(snapshot("first", ToolStatus::Running), 1));
        assert!(store.update(snapshot("third", ToolStatus::Running), 3));
        assert!(!store.update(snapshot("late second", ToolStatus::Running), 2));
        assert!(store.finish(snapshot("complete output", ToolStatus::Succeeded), 4));
        let latest = store.get("call").unwrap();
        assert_eq!(latest.output, "complete output");
        assert_eq!(latest.status, ToolStatus::Succeeded);
    }
}
