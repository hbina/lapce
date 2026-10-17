use crate::{main_split::SplitInfo, panel::data::PanelInfo};

pub use lapce_client::workspace::{LapceWorkspace, LapceWorkspaceType, SshHost};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub split: SplitInfo,
    pub panel: PanelInfo,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_breakpoints_are_discarded_without_losing_workspace_layout() {
        let legacy = serde_json::json!({
            "split": { "children": [], "direction": "Vertical" },
            "panel": {
                "panels": {}, "styles": {}, "sections": {},
                "size": { "left": 200.0, "left_split": 0.5, "bottom": 100.0,
                    "bottom_split": 0.5, "right": 200.0, "right_split": 0.5 }
            },
            "breakpoints": { "/tmp/old.rs": [{ "line": 12, "enabled": true }] }
        });
        let workspace: WorkspaceInfo =
            serde_json::from_value(legacy.clone()).unwrap();
        let saved = serde_json::to_value(workspace).unwrap();
        assert_eq!(saved["split"], legacy["split"]);
        assert_eq!(saved["panel"], legacy["panel"]);
        assert!(saved.get("breakpoints").is_none());
    }
}
