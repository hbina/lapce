use std::{path::PathBuf, rc::Rc};

use crate::reactive_model::ModelField;
use floem::{
    keyboard::Modifiers,
    reactive::{RwSignal, SignalUpdate},
};
use indexmap::IndexMap;
use lapce_core::mode::Mode;
use lapce_engine::workspace::WorkspaceActions;
use lapce_rpc::source_control::{DiffInfo, FileDiff};

use crate::{
    command::CommandExecuted,
    keypress::{KeyPressFocus, condition::Condition},
    window_tab::CommonData,
};

#[derive(Clone, Debug)]
pub struct SourceControlData {
    // Changed files available for read-only diff navigation.
    pub file_diffs: ModelField<WorkspaceActions, IndexMap<PathBuf, FileDiff>>,
    pub branch: ModelField<WorkspaceActions, String>,
    pub branches: ModelField<WorkspaceActions, im::Vector<String>>,
    pub tags: ModelField<WorkspaceActions, im::Vector<String>>,
    state: RwSignal<WorkspaceActions>,
    changed: RwSignal<()>,
    pub common: Rc<CommonData>,
}

impl KeyPressFocus for SourceControlData {
    fn get_mode(&self) -> Mode {
        Mode::Normal
    }

    fn check_condition(&self, condition: Condition) -> bool {
        matches!(
            condition,
            Condition::PanelFocus | Condition::SourceControlFocus
        )
    }

    fn run_command(
        &self,
        _command: &crate::command::LapceCommand,
        _count: Option<usize>,
        _mods: Modifiers,
    ) -> CommandExecuted {
        CommandExecuted::No
    }
    fn receive_char(&self, _c: &str) {}
}

impl SourceControlData {
    pub fn new(common: Rc<CommonData>) -> Self {
        let state = common.workspace_actions;
        let changed = common.workspace_changed;
        Self {
            state,
            changed,
            file_diffs: ModelField::new(
                state,
                changed,
                |state| &state.source_control.file_diffs,
                |state| &mut state.source_control.file_diffs,
                None,
            ),
            branch: ModelField::new(
                state,
                changed,
                |state| &state.source_control.branch,
                |state| &mut state.source_control.branch,
                None,
            ),
            branches: ModelField::new(
                state,
                changed,
                |state| &state.source_control.branches,
                |state| &mut state.source_control.branches,
                None,
            ),
            tags: ModelField::new(
                state,
                changed,
                |state| &state.source_control.tags,
                |state| &mut state.source_control.tags,
                None,
            ),
            common,
        }
    }

    pub fn apply_diff(&self, diff: &DiffInfo) {
        if self
            .state
            .try_update(|state| state.source_control.apply_diff(diff))
            .is_some()
        {
            self.changed.set(());
        }
    }
}
