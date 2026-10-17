use std::{rc::Rc, sync::Arc};

use floem::reactive::{RwSignal, Scope, SignalGet, SignalUpdate, SignalWith};
use lapce_core::mode::Mode;
use lapce_rpc::terminal::{TermId, TerminalProfile};

use super::{data::TerminalData, tab::TerminalTabData};
use crate::{
    id::TerminalTabId,
    keypress::{EventRef, KeyPressData, KeyPressFocus, KeyPressHandle},
    panel::kind::PanelKind,
    window_tab::{CommonData, Focus},
    workspace::LapceWorkspace,
};

pub struct TerminalTabInfo {
    pub active: usize,
    pub tabs: im::Vector<(RwSignal<usize>, TerminalTabData)>,
}

#[derive(Clone)]
pub struct TerminalPanelData {
    pub cx: Scope,
    pub workspace: Arc<LapceWorkspace>,
    pub tab_info: RwSignal<TerminalTabInfo>,
    pub common: Rc<CommonData>,
}

impl TerminalPanelData {
    pub fn new(
        workspace: Arc<LapceWorkspace>,
        profile: Option<TerminalProfile>,
        common: Rc<CommonData>,
    ) -> Self {
        let terminal_tab =
            TerminalTabData::new(workspace.clone(), profile, common.clone());

        let cx = common.scope;

        let tabs =
            im::vector![(terminal_tab.scope.create_rw_signal(0), terminal_tab)];
        let tab_info = TerminalTabInfo { active: 0, tabs };
        let tab_info = cx.create_rw_signal(tab_info);

        Self {
            cx,
            workspace,
            tab_info,
            common,
        }
    }

    pub fn active_tab(&self, tracked: bool) -> Option<TerminalTabData> {
        if tracked {
            self.tab_info.with(|info| {
                info.tabs
                    .get(info.active)
                    .or_else(|| info.tabs.last())
                    .cloned()
                    .map(|(_, tab)| tab)
            })
        } else {
            self.tab_info.with_untracked(|info| {
                info.tabs
                    .get(info.active)
                    .or_else(|| info.tabs.last())
                    .cloned()
                    .map(|(_, tab)| tab)
            })
        }
    }

    pub fn key_down<'a>(
        &self,
        event: impl Into<EventRef<'a>> + Copy,
        keypress: &KeyPressData,
    ) -> Option<KeyPressHandle> {
        if self.tab_info.with_untracked(|info| info.tabs.is_empty()) {
            self.new_tab(None);
        }

        let tab = self.active_tab(false);
        let terminal = tab.and_then(|tab| tab.active_terminal(false));
        if let Some(terminal) = terminal {
            let handle = keypress.key_down(event, &terminal);
            let mode = terminal.get_mode();

            if !handle.handled && mode == Mode::Terminal {
                if let EventRef::Keyboard(key_event) = event.into() {
                    if terminal.send_keypress(key_event) {
                        return Some(KeyPressHandle {
                            handled: true,
                            keymatch: handle.keymatch,
                            keypress: handle.keypress,
                        });
                    }
                }
            }
            Some(handle)
        } else {
            None
        }
    }

    pub fn new_tab(&self, profile: Option<TerminalProfile>) {
        let terminal_tab = TerminalTabData::new(
            self.workspace.clone(),
            profile,
            self.common.clone(),
        );

        self.tab_info.update(|info| {
            info.tabs.insert(
                if info.tabs.is_empty() {
                    0
                } else {
                    (info.active + 1).min(info.tabs.len())
                },
                (terminal_tab.scope.create_rw_signal(0), terminal_tab.clone()),
            );
            let new_active = (info.active + 1).min(info.tabs.len() - 1);
            info.active = new_active;
        });
    }

    pub fn next_tab(&self) {
        self.tab_info.update(|info| {
            if info.active >= info.tabs.len().saturating_sub(1) {
                info.active = 0;
            } else {
                info.active += 1;
            }
        });
    }

    pub fn previous_tab(&self) {
        self.tab_info.update(|info| {
            if info.active == 0 {
                info.active = info.tabs.len().saturating_sub(1);
            } else {
                info.active -= 1;
            }
        });
    }

    pub fn close_tab(&self, terminal_tab_id: Option<TerminalTabId>) {
        if let Some(close_tab) = self
            .tab_info
            .try_update(|info| {
                let mut close_tab = None;
                if let Some(terminal_tab_id) = terminal_tab_id {
                    if let Some(index) =
                        info.tabs.iter().enumerate().find_map(|(index, (_, t))| {
                            if t.terminal_tab_id == terminal_tab_id {
                                Some(index)
                            } else {
                                None
                            }
                        })
                    {
                        close_tab = Some(
                            info.tabs.remove(index).1.terminals.get_untracked(),
                        );
                    }
                } else {
                    let active = info.active.min(info.tabs.len().saturating_sub(1));
                    if !info.tabs.is_empty() {
                        info.tabs.remove(active);
                    }
                }
                let new_active = info.active.min(info.tabs.len().saturating_sub(1));
                info.active = new_active;
                close_tab
            })
            .flatten()
        {
            for (_, data) in close_tab {
                data.stop();
            }
        }
    }

    pub fn set_title(&self, term_id: &TermId, title: &str) {
        if let Some(t) = self.get_terminal(term_id) {
            t.title.set(title.to_string());
        }
    }

    pub fn get_terminal(&self, term_id: &TermId) -> Option<TerminalData> {
        self.tab_info.with_untracked(|info| {
            for (_, tab) in &info.tabs {
                let terminal = tab.terminals.with_untracked(|terminals| {
                    terminals
                        .iter()
                        .find(|(_, t)| &t.term_id == term_id)
                        .cloned()
                });
                if let Some(terminal) = terminal {
                    return Some(terminal.1);
                }
            }
            None
        })
    }

    fn get_terminal_in_tab(
        &self,
        term_id: &TermId,
    ) -> Option<(usize, TerminalTabData, usize, TerminalData)> {
        self.tab_info.with_untracked(|info| {
            for (tab_index, (_, tab)) in info.tabs.iter().enumerate() {
                let result = tab.terminals.with_untracked(|terminals| {
                    terminals
                        .iter()
                        .enumerate()
                        .find(|(_, (_, t))| &t.term_id == term_id)
                        .map(|(i, (_, terminal))| (i, terminal.clone()))
                });
                if let Some((index, terminal)) = result {
                    return Some((tab_index, tab.clone(), index, terminal));
                }
            }
            None
        })
    }

    pub fn split(&self, term_id: TermId) {
        if let Some((_, tab, index, _)) = self.get_terminal_in_tab(&term_id) {
            let terminal_data = TerminalData::new(
                tab.scope,
                self.workspace.clone(),
                None,
                self.common.clone(),
            );
            let i = terminal_data.scope.create_rw_signal(0);
            tab.terminals.update(|terminals| {
                terminals.insert(index + 1, (i, terminal_data));
            });
        }
    }

    pub fn split_next(&self, term_id: TermId) {
        if let Some((_, tab, index, _)) = self.get_terminal_in_tab(&term_id) {
            let max = tab.terminals.with_untracked(|t| t.len() - 1);
            let new_index = (index + 1).min(max);
            if new_index != index {
                tab.active.set(new_index);
            }
        }
    }

    pub fn split_previous(&self, term_id: TermId) {
        if let Some((_, tab, index, _)) = self.get_terminal_in_tab(&term_id) {
            let new_index = index.saturating_sub(1);
            if new_index != index {
                tab.active.set(new_index);
            }
        }
    }

    pub fn split_exchange(&self, term_id: TermId) {
        if let Some((_, tab, index, _)) = self.get_terminal_in_tab(&term_id) {
            let max = tab.terminals.with_untracked(|t| t.len() - 1);
            if index < max {
                tab.terminals.update(|terminals| {
                    terminals.swap(index, index + 1);
                });
            }
        }
    }

    pub fn close_terminal(&self, term_id: &TermId) {
        if let Some((_, tab, index, _)) = self.get_terminal_in_tab(term_id) {
            let active = tab.active.get_untracked();
            let len = tab
                .terminals
                .try_update(|terminals| {
                    terminals.remove(index);
                    terminals.len()
                })
                .unwrap();
            if len == 0 {
                self.close_tab(Some(tab.terminal_tab_id));
            } else {
                let new_active = active.min(len.saturating_sub(1));
                if new_active != active {
                    tab.active.set(new_active);
                }
            }
        }
    }

    pub fn launch_failed(&self, term_id: &TermId, error: &str) {
        if let Some(terminal) = self.get_terminal(term_id) {
            terminal.observe_failure(error);
        }
    }

    pub fn terminal_stopped(&self, term_id: &TermId, exit_code: Option<i32>) {
        if let Some(terminal) = self.get_terminal(term_id) {
            if !terminal.observe_exit(exit_code) {
                return;
            }
            self.close_terminal(term_id);
        }
    }

    pub fn focus_terminal(&self, term_id: TermId) {
        if let Some((tab_index, terminal_tab, index, _terminal)) =
            self.get_terminal_in_tab(&term_id)
        {
            self.tab_info.update(|info| {
                info.active = tab_index;
            });
            terminal_tab.active.set(index);
            self.common.focus.set(Focus::Panel(PanelKind::Terminal));
        }
    }

    pub fn set_process_id(&self, term_id: &TermId, process_id: Option<u32>) {
        if let Some(terminal) = self.get_terminal(term_id) {
            terminal.observe_process_id(process_id);
        }
    }
}
