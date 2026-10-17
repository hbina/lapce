use super::model::{FileNodeItem, FileNodeViewKind};

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

use floem::{
    action::show_context_menu,
    event::EventPropagation,
    ext_event::create_ext_action,
    keyboard::Modifiers,
    menu::{Menu, MenuItem},
    reactive::{ReadSignal, RwSignal, Scope, SignalGet, SignalUpdate, SignalWith},
    views::editor::text::SystemClipboard,
};
use globset::Glob;
use lapce_core::{mode::Mode, register::Clipboard};
use lapce_rpc::proxy::PathRequest;

use crate::{
    command::{CommandExecuted, InternalCommand, LapceCommand},
    config::LapceConfig,
    keypress::{KeyPressFocus, condition::Condition},
    window_tab::CommonData,
};

#[derive(Clone, Debug)]
pub struct FileExplorerData {
    pub root: RwSignal<FileNodeItem>,
    pub common: Rc<CommonData>,
    pub scroll_to_line: RwSignal<Option<f64>>,
    left_diff_path: RwSignal<Option<PathBuf>>,
    pub select: RwSignal<Option<FileNodeViewKind>>,
}

impl KeyPressFocus for FileExplorerData {
    fn get_mode(&self) -> Mode {
        Mode::Normal
    }
    fn check_condition(&self, condition: Condition) -> bool {
        matches!(
            condition,
            Condition::PanelFocus | Condition::FileExplorerFocus
        )
    }
    fn run_command(
        &self,
        _: &LapceCommand,
        _: Option<usize>,
        _: Modifiers,
    ) -> CommandExecuted {
        CommandExecuted::No
    }
    fn receive_char(&self, _: &str) {}
}

impl FileExplorerData {
    pub fn new(cx: Scope, common: Rc<CommonData>) -> Self {
        let path = common.workspace.path.clone().unwrap_or_default();
        let root = cx.create_rw_signal(FileNodeItem {
            path: path.clone(),
            is_dir: true,
            read: false,
            open: false,
            children: HashMap::new(),
            children_open_count: 0,
        });
        let data = Self {
            root,
            common,
            scroll_to_line: cx.create_rw_signal(None),
            left_diff_path: cx.create_rw_signal(None),
            select: cx.create_rw_signal(None),
        };
        if data.common.workspace.path.is_some() {
            // only fill in the child files if there is open folder
            data.toggle_expand(&path);
        }
        data
    }

    /// Reload the file explorer data via reading the root directory.
    /// Note that this will not update immediately.
    pub fn reload(&self) {
        let path = self.root.with_untracked(|root| root.path.clone());
        self.read_dir(&path);
    }

    /// Toggle whether the directory is expanded or not.
    /// Does nothing if the path does not exist or is not a directory.
    pub fn toggle_expand(&self, path: &Path) {
        if self
            .root
            .try_update(|root| root.toggle_directory(path))
            .flatten()
            == Some(true)
        {
            self.read_dir(path);
        }
    }

    pub fn read_dir(&self, path: &Path) {
        self.read_dir_cb(path, |_| {});
    }

    /// Read the directory's information and update the file explorer tree.
    /// `done : FnOnce(was_read: bool)` is called when the operation is completed, whether success,
    /// failure, or ignored.
    pub fn read_dir_cb(&self, path: &Path, done: impl FnOnce(bool) + 'static) {
        let root = self.root;
        let data = self.clone();
        let config = self.common.config;
        let query = lapce_engine::directory::DirectoryQuery::new(path.to_path_buf());
        let send = {
            let path = path.to_path_buf();
            let query = query.clone();
            create_ext_action(self.common.scope, move |result| {
                let Ok(mut items) = query.finish(result) else {
                    done(false);
                    return;
                };

                match Glob::new(&config.get().editor.files_exclude) {
                    Ok(glob) => {
                        let matcher = glob.compile_matcher();
                        items.retain(|item| !matcher.is_match(&item.path));
                    }
                    Err(error) => {
                        tracing::error!(target: "files_exclude", "Failed to compile glob: {error}")
                    }
                }
                let refresh = root
                    .try_update(|root| root.refresh_directory(&path, items))
                    .flatten();
                let accepted = refresh.is_some();
                let refresh = refresh.unwrap_or_default();
                for path in refresh {
                    data.read_dir(&path);
                }
                done(accepted);
            })
        };

        // Ask the proxy for the directory information
        self.common
            .proxy
            .read_dir(PathRequest::Path(path.to_path_buf()), send);
    }

    /// Returns `true` if `path` exists in the file explorer tree and is a directory, `false`
    /// otherwise.
    fn is_dir(&self, path: &Path) -> bool {
        self.root.with_untracked(|root| {
            root.get_file_node(path).is_some_and(|node| node.is_dir)
        })
    }

    pub fn click(&self, path: &Path, config: ReadSignal<Arc<LapceConfig>>) {
        if self.is_dir(path) {
            self.toggle_expand(path);
        } else if !config.get_untracked().core.file_explorer_double_click {
            self.common
                .internal_command
                .send(InternalCommand::OpenFile {
                    path: path.to_path_buf(),
                })
        }
    }

    pub fn reveal_in_file_tree(&self, path: PathBuf) {
        use lapce_engine::explorer::RevealStep;
        let step = self.root.try_update(|root| root.reveal(&path));
        match step {
            Some(RevealStep::Visible { row }) => {
                self.scroll_to_line.set(Some(row as f64));
                self.select.set(Some(FileNodeViewKind::Path(path)));
            }
            Some(RevealStep::ReadDirectory(directory)) => {
                let explorer = self.clone();
                self.read_dir_cb(&directory, move |accepted| {
                    if accepted {
                        explorer.reveal_in_file_tree(path);
                    }
                });
            }
            _ => {}
        }
    }

    pub fn double_click(
        &self,
        path: &Path,
        config: ReadSignal<Arc<LapceConfig>>,
    ) -> EventPropagation {
        if self.is_dir(path) {
            EventPropagation::Continue
        } else if config.get_untracked().core.file_explorer_double_click {
            self.common.internal_command.send(
                InternalCommand::OpenAndConfirmedFile {
                    path: path.to_path_buf(),
                },
            );
            EventPropagation::Stop
        } else {
            self.common
                .internal_command
                .send(InternalCommand::MakeConfirmed);
            EventPropagation::Stop
        }
    }

    pub fn secondary_click(&self, path: &Path) {
        let common = self.common.clone();
        let path_a = path.to_owned();
        let left_diff_path = self.left_diff_path;

        let Some(_) = self.common.workspace.path.as_ref() else {
            // There is no context menu if we are not in a workspace
            return;
        };

        let mut menu = Menu::new("");

        // TODO: there are situations where we can open the file explorer to remote files
        if !common.workspace.kind.is_remote() {
            let path = path_a.clone();

            let title = "Reveal in system file explorer";

            menu = menu.entry(MenuItem::new(title).action(move || {
                let path = path.parent().unwrap_or(&path);
                if !path.exists() {
                    return;
                }

                if let Err(err) = open::that(path) {
                    tracing::error!(
                        "Failed to reveal file in system file explorer: {}",
                        err
                    );
                }
            }));
        }

        menu = menu.separator();

        let path = path_a.clone();
        menu = menu.entry(MenuItem::new("Copy Path").action(move || {
            let mut clipboard = SystemClipboard::new();
            clipboard.put_string(path.to_string_lossy());
        }));

        let path = path_a.clone();
        let workspace = common.workspace.clone();
        menu = menu.entry(MenuItem::new("Copy Relative Path").action(move || {
            let relative_path = if let Some(workspace_path) = &workspace.path {
                path.strip_prefix(workspace_path).unwrap_or(&path)
            } else {
                path.as_ref()
            };

            let mut clipboard = SystemClipboard::new();
            clipboard.put_string(relative_path.to_string_lossy());
        }));

        menu = menu.separator();

        let path = path_a.clone();
        menu = menu.entry(
            MenuItem::new("Select for Compare")
                .action(move || left_diff_path.set(Some(path.clone()))),
        );

        if let Some(left_path) = self.left_diff_path.get_untracked() {
            let common = self.common.clone();
            let right_path = path_a.to_owned();

            menu = menu.entry(MenuItem::new("Compare with Selected").action(
                move || {
                    common
                        .internal_command
                        .send(InternalCommand::OpenDiffFiles {
                            left_path: left_path.clone(),
                            right_path: right_path.clone(),
                        })
                },
            ))
        }

        menu = menu.separator();

        let internal_command = common.internal_command;
        menu = menu.entry(MenuItem::new("Refresh").action(move || {
            internal_command.send(InternalCommand::ReloadFileExplorer);
        }));

        show_context_menu(menu, None);
    }

    pub fn middle_click(&self, path: &Path) -> EventPropagation {
        if self.is_dir(path) {
            EventPropagation::Continue
        } else {
            self.common
                .internal_command
                .send(InternalCommand::OpenFileInNewTab {
                    path: path.to_path_buf(),
                });
            EventPropagation::Stop
        }
    }
}
