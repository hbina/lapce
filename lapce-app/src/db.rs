use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

use anyhow::{Result, anyhow};
use crossbeam_channel::{Sender, unbounded};
use floem::{peniko::kurbo::Vec2, reactive::SignalGet};
use lapce_core::directory::Directory;
use sha2::{Digest, Sha256};

use crate::{
    app::AppData,
    doc::DocInfo,
    panel::{data::PanelOrder, kind::PanelKind},
    window::{WindowData, WindowInfo},
    window_tab::WindowTabData,
    workspace::{LapceWorkspace, WorkspaceInfo},
};

const WINDOW: &str = "window";
const WORKSPACE_INFO: &str = "workspace_info";
const WORKSPACE_FILES: &str = "workspace_files";
const PANEL_ORDERS: &str = "panel_orders";

pub enum SaveEvent {
    Window(WindowInfo),
    Workspace(LapceWorkspace, WorkspaceInfo),
    Doc(DocInfo),
    PanelOrder(PanelOrder),
}

#[derive(Clone)]
pub struct LapceDb {
    folder: PathBuf,
    workspace_folder: PathBuf,
    save_tx: Sender<SaveEvent>,
}

impl LapceDb {
    pub fn new() -> Result<Self> {
        let folder = Directory::config_directory()
            .ok_or_else(|| anyhow!("can't get config directory"))?
            .join("db");
        let workspace_folder = folder.join("workspaces");
        if let Err(err) = std::fs::create_dir_all(&workspace_folder) {
            tracing::error!("{:?}", err);
        }

        let (save_tx, save_rx) = unbounded();

        let db = Self {
            save_tx,
            workspace_folder,
            folder,
        };
        let local_db = db.clone();
        std::thread::Builder::new()
            .name("SaveEventHandler".to_owned())
            .spawn(move || -> Result<()> {
                loop {
                    let event = save_rx.recv()?;
                    match event {
                        SaveEvent::Window(info) => {
                            if let Err(err) = local_db.insert_window_info(info) {
                                tracing::error!("{:?}", err);
                            }
                        }
                        SaveEvent::Workspace(workspace, info) => {
                            if let Err(err) =
                                local_db.insert_workspace(&workspace, &info)
                            {
                                tracing::error!("{:?}", err);
                            }
                        }

                        SaveEvent::Doc(info) => {
                            if let Err(err) = local_db.insert_doc(&info) {
                                tracing::error!("{:?}", err);
                            }
                        }

                        SaveEvent::PanelOrder(order) => {
                            if let Err(err) = local_db.insert_panel_orders(&order) {
                                tracing::error!("{:?}", err);
                            }
                        }
                    }
                }
            })
            .unwrap();
        Ok(db)
    }

    pub fn save_window_tab(&self, data: Rc<WindowTabData>) -> Result<()> {
        let workspace = (*data.workspace).clone();
        let workspace_info = data.workspace_info();

        self.save_tx
            .send(SaveEvent::Workspace(workspace, workspace_info))?;
        // self.insert_unsaved_buffer(main_split)?;

        Ok(())
    }

    pub fn get_workspace_info(
        &self,
        workspace: &LapceWorkspace,
    ) -> Result<WorkspaceInfo> {
        let info = std::fs::read_to_string(
            self.workspace_folder
                .join(workspace_folder_name(workspace))
                .join(WORKSPACE_INFO),
        )?;
        let info: WorkspaceInfo = serde_json::from_str(&info)?;
        Ok(info)
    }

    fn insert_workspace(
        &self,
        workspace: &LapceWorkspace,
        info: &WorkspaceInfo,
    ) -> Result<()> {
        let folder = self.workspace_folder.join(workspace_folder_name(workspace));
        if let Err(err) = std::fs::create_dir_all(&folder) {
            tracing::error!("{:?}", err);
        }
        let workspace_info = serde_json::to_string_pretty(info)?;
        std::fs::write(folder.join(WORKSPACE_INFO), workspace_info)?;
        Ok(())
    }

    pub fn save_app(&self, data: &AppData) -> Result<()> {
        let windows = data.windows.get_untracked();
        for window in windows.values() {
            self.save_window(window.clone())?;
        }
        if let Some(window) = windows
            .get(&data.active_window.get_untracked())
            .or_else(|| windows.values().next())
        {
            self.save_tx.send(SaveEvent::Window(window.info()))?;
        }
        Ok(())
    }

    fn insert_window_info(&self, info: WindowInfo) -> Result<()> {
        std::fs::write(
            self.folder.join(WINDOW),
            serde_json::to_string_pretty(&info)?,
        )?;
        Ok(())
    }

    pub fn insert_app(&self, data: AppData) -> Result<()> {
        let windows = data.windows.get_untracked();
        for window in windows.values() {
            self.insert_window(window.clone())?;
        }
        if let Some(window) = windows
            .get(&data.active_window.get_untracked())
            .or_else(|| windows.values().next())
        {
            self.insert_window_info(window.info())?;
        }
        Ok(())
    }

    pub fn get_window(&self) -> Result<WindowInfo> {
        let info = std::fs::read_to_string(self.folder.join(WINDOW))?;
        let mut info: WindowInfo = serde_json::from_str(&info)?;
        if info.size.width < 10.0 {
            info.size.width = 800.0;
        }
        if info.size.height < 10.0 {
            info.size.height = 600.0;
        }
        Ok(info)
    }

    pub fn save_window(&self, data: WindowData) -> Result<()> {
        self.save_window_tab(data.session.get_untracked())
    }

    pub fn insert_window(&self, data: WindowData) -> Result<()> {
        self.insert_window_tab(data.session.get_untracked())?;
        self.insert_window_info(data.info())
    }

    pub fn insert_window_tab(&self, data: Rc<WindowTabData>) -> Result<()> {
        let workspace = (*data.workspace).clone();
        let workspace_info = data.workspace_info();

        self.insert_workspace(&workspace, &workspace_info)?;
        // self.insert_unsaved_buffer(main_split)?;

        Ok(())
    }

    pub fn get_panel_orders(&self) -> Result<PanelOrder> {
        let panel_orders = std::fs::read_to_string(self.folder.join(PANEL_ORDERS))?;
        let mut deserializer = serde_json::Deserializer::from_str(&panel_orders);
        let mut panel_orders =
            crate::panel::data::deserialize_panel_order(&mut deserializer)?;

        use strum::IntoEnumIterator;
        for kind in PanelKind::iter() {
            if kind.position(&panel_orders).is_none() {
                let panels =
                    panel_orders.entry(kind.default_position()).or_default();
                panels.push_back(kind);
            }
        }

        Ok(panel_orders)
    }

    pub fn save_panel_orders(&self, order: PanelOrder) {
        if let Err(err) = self.save_tx.send(SaveEvent::PanelOrder(order)) {
            tracing::error!("{:?}", err);
        }
    }

    fn insert_panel_orders(&self, order: &PanelOrder) -> Result<()> {
        let info = serde_json::to_string_pretty(order)?;
        std::fs::write(self.folder.join(PANEL_ORDERS), info)?;
        Ok(())
    }

    pub fn save_doc_position(
        &self,
        workspace: &LapceWorkspace,
        path: PathBuf,
        cursor_offset: usize,
        scroll_offset: Vec2,
    ) {
        let info = DocInfo {
            workspace: workspace.clone(),
            path,
            scroll_offset: (scroll_offset.x, scroll_offset.y),
            cursor_offset,
        };
        if let Err(err) = self.save_tx.send(SaveEvent::Doc(info)) {
            tracing::error!("{:?}", err);
        }
    }

    fn insert_doc(&self, info: &DocInfo) -> Result<()> {
        let folder = self
            .workspace_folder
            .join(workspace_folder_name(&info.workspace))
            .join(WORKSPACE_FILES);
        if let Err(err) = std::fs::create_dir_all(&folder) {
            tracing::error!("{:?}", err);
        }
        let contents = serde_json::to_string_pretty(info)?;
        std::fs::write(folder.join(doc_path_name(&info.path)), contents)?;
        Ok(())
    }

    pub fn get_doc_info(
        &self,
        workspace: &LapceWorkspace,
        path: &Path,
    ) -> Result<DocInfo> {
        let folder = self
            .workspace_folder
            .join(workspace_folder_name(workspace))
            .join(WORKSPACE_FILES);
        let info = std::fs::read_to_string(folder.join(doc_path_name(path)))?;
        let info: DocInfo = serde_json::from_str(&info)?;
        Ok(info)
    }
}

fn workspace_folder_name(workspace: &LapceWorkspace) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .append_key_only(&workspace.to_string())
        .finish()
}

fn doc_path_name(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    format!("{:x}", hasher.finalize())
}
