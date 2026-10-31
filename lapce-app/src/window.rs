use std::{rc::Rc, sync::Arc};

use floem::{
    ViewId,
    action::TimerToken,
    peniko::kurbo::{Point, Size},
    reactive::{ReadSignal, RwSignal, Scope, SignalGet, SignalUpdate, use_context},
    window::WindowId,
};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppCommand, command::WindowCommand, config::LapceConfig, db::LapceDb,
    keypress::EventRef, listener::Listener, update::ReleaseInfo,
    window_tab::WindowTabData, workspace::LapceWorkspace,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub size: Size,
    pub pos: Point,
    pub maximised: bool,
    #[serde(default)]
    pub root: LapceWorkspace,
}

#[derive(Clone)]
pub struct WindowCommonData {
    pub window_command: Listener<WindowCommand>,
    pub window_scale: RwSignal<f64>,
    pub size: RwSignal<Size>,
    pub window_maximized: RwSignal<bool>,
    pub latest_release: ReadSignal<Arc<Option<ReleaseInfo>>>,
    pub ime_allowed: RwSignal<bool>,
    pub cursor_blink_timer: RwSignal<TimerToken>,
    // the value to be update by cursor blinking
    pub hide_cursor: RwSignal<bool>,
    pub app_view_id: RwSignal<ViewId>,
}

/// `WindowData` is the application model for a top-level window.
///
/// A top-level window can be independently moved around and
/// resized using your window manager. Normally Lapce has only one
/// top-level window, but new ones can be created using the "New Window"
/// command.
///
/// Each window owns one root/session, its size, and its position.
#[derive(Clone)]
pub struct WindowData {
    pub window_id: WindowId,
    pub scope: Scope,
    pub session: RwSignal<Rc<WindowTabData>>,
    pub app_command: Listener<AppCommand>,
    pub position: RwSignal<Point>,
    pub root_view_id: RwSignal<ViewId>,
    pub window_scale: RwSignal<f64>,
    pub config: RwSignal<Arc<LapceConfig>>,
    pub ime_enabled: RwSignal<bool>,
    pub common: Rc<WindowCommonData>,
}

impl WindowData {
    pub fn new(
        window_id: WindowId,
        app_view_id: RwSignal<ViewId>,
        info: WindowInfo,
        window_scale: RwSignal<f64>,
        latest_release: ReadSignal<Arc<Option<ReleaseInfo>>>,
        app_command: Listener<AppCommand>,
    ) -> Self {
        let cx = Scope::new();
        let config = LapceConfig::load(&LapceWorkspace::default());
        let config = cx.create_rw_signal(Arc::new(config));
        let root_view_id = cx.create_rw_signal(ViewId::new());

        let window_command = Listener::new_empty(cx);
        let ime_allowed = cx.create_rw_signal(false);
        let window_maximized = cx.create_rw_signal(false);
        let size = cx.create_rw_signal(Size::ZERO);
        let cursor_blink_timer = cx.create_rw_signal(TimerToken::INVALID);
        let hide_cursor = cx.create_rw_signal(false);

        let common = Rc::new(WindowCommonData {
            window_command,
            window_scale,
            size,
            window_maximized,
            latest_release,
            ime_allowed,
            cursor_blink_timer,
            hide_cursor,
            app_view_id,
        });

        let session =
            Rc::new(WindowTabData::new(cx, Arc::new(info.root), common.clone()));
        let session = cx.create_rw_signal(session);
        let position = cx.create_rw_signal(info.pos);

        let window_data = Self {
            window_id,
            scope: cx,
            session,
            position,
            root_view_id,
            window_scale,
            app_command,
            config,
            ime_enabled: cx.create_rw_signal(false),
            common,
        };

        {
            let window_data = window_data.clone();
            window_data.common.window_command.listen(move |cmd| {
                window_data.run_window_command(cmd);
            });
        }

        window_data
    }

    pub fn reload_config(&self) {
        let config = LapceConfig::load(&LapceWorkspace::default());
        self.config.set(Arc::new(config));
        self.session.get_untracked().reload_config();
    }

    pub fn run_window_command(&self, cmd: WindowCommand) {
        match cmd {
            WindowCommand::OpenRootWindow { root } => {
                self.app_command
                    .send(AppCommand::NewWindow { root: Some(root) });
            }
            WindowCommand::ReloadRoot => {
                let old = self.session.get_untracked();
                let db: Arc<LapceDb> = use_context().unwrap();
                if let Err(error) = db.insert_window_tab(old.clone()) {
                    tracing::error!(%error, "Could not save root layout");
                }
                old.proxy.shutdown();
                let next = Rc::new(WindowTabData::new(
                    self.scope,
                    old.workspace.clone(),
                    self.common.clone(),
                ));
                self.session.set(next);
                old.scope.dispose();
            }
            WindowCommand::NewWindow => {
                self.app_command.send(AppCommand::NewWindow { root: None })
            }
            WindowCommand::CloseWindow => self
                .app_command
                .send(AppCommand::CloseWindow(self.window_id)),
        }
        self.app_command.send(AppCommand::SaveApp);
    }

    pub fn key_down<'a>(&self, event: impl Into<EventRef<'a>> + Copy) -> bool {
        self.session.get_untracked().key_down(event)
    }

    pub fn info(&self) -> WindowInfo {
        WindowInfo {
            size: self.common.size.get_untracked(),
            pos: self.position.get_untracked(),
            maximised: false,
            root: (*self.session.get_untracked().workspace).clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_workspace_lists_are_ignored_but_window_geometry_survives() {
        let old = serde_json::json!({
            "size": {"width": 1000.0, "height": 700.0},
            "pos": {"x": 40.0, "y": 50.0}, "maximised": false,
            "tabs": {"active_tab": 1, "workspaces": [
                {"kind": "Local", "path": "/old/first", "last_open": 1},
                {"kind": "Local", "path": "/old/second", "last_open": 2}
            ]}
        });
        let window: WindowInfo = serde_json::from_value(old).unwrap();
        assert_eq!(window.size, Size::new(1000.0, 700.0));
        assert_eq!(window.pos, Point::new(40.0, 50.0));
        assert!(window.root.path.is_none());
        let saved = serde_json::to_value(window).unwrap();
        assert!(saved.get("tabs").is_none());
        assert!(saved["root"].get("last_open").is_none());
    }

    #[test]
    fn single_local_or_remote_root_roundtrips_without_a_workspace_list() {
        use crate::workspace::{LapceWorkspaceType, SshHost};
        for kind in [
            LapceWorkspaceType::Local,
            LapceWorkspaceType::RemoteSSH(SshHost::from_string("user@host:2222")),
        ] {
            let window = WindowInfo {
                size: Size::new(900.0, 600.0),
                pos: Point::ZERO,
                maximised: false,
                root: LapceWorkspace {
                    kind,
                    path: Some("/source".into()),
                },
            };
            let saved = serde_json::to_value(&window).unwrap();
            assert!(saved.get("tabs").is_none());
            let loaded: WindowInfo = serde_json::from_value(saved).unwrap();
            assert_eq!(loaded.root, window.root);
        }
    }
}
