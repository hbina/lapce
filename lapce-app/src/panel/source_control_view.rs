use std::{path::PathBuf, rc::Rc};

use floem::{
    View,
    event::EventListener,
    peniko::kurbo::Rect,
    reactive::{SignalGet, SignalUpdate, create_memo, create_rw_signal},
    style::{CursorStyle, Style},
    views::{Decorators, container, dyn_stack, label, scroll, stack, svg, text},
};
use lapce_rpc::source_control::FileDiff;

use super::{
    data::PanelSection, kind::PanelKind, position::PanelPosition,
    view::foldable_panel_section,
};
use crate::{
    command::InternalCommand,
    config::{color::LapceColor, icon::LapceIcons},
    source_control::SourceControlData,
    window_tab::{Focus, WindowTabData},
};

pub fn source_control_panel(
    window_tab_data: Rc<WindowTabData>,
    _position: PanelPosition,
) -> impl View {
    let config = window_tab_data.common.config;
    let focus = window_tab_data.common.focus;
    foldable_panel_section(
        text("Changes"),
        file_diffs_view(window_tab_data.source_control.clone()),
        window_tab_data.panel.section_open(PanelSection::Changes),
        config,
    )
    .on_event_stop(EventListener::PointerDown, move |_| {
        focus.set(Focus::Panel(PanelKind::SourceControl));
    })
    .style(|s| s.flex_col().size_pct(100.0, 100.0))
    .debug_name("Repository Changes Panel")
}

fn file_diffs_view(source_control: SourceControlData) -> impl View {
    let file_diffs = source_control.file_diffs;
    let config = source_control.common.config;
    let workspace = source_control.common.workspace.clone();
    let panel_rect = create_rw_signal(Rect::ZERO);
    let panel_width = create_memo(move |_| panel_rect.get().width());
    let internal_command = source_control.common.internal_command;

    let view_fn = move |(path, diff): (PathBuf, FileDiff)| {
        let diff_for_style = diff.clone();
        let full_path = path.clone();
        let path_for_click = full_path.clone();

        let path = path
            .strip_prefix(&workspace.path)
            .unwrap_or(&full_path)
            .to_path_buf();
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let folder = path
            .parent()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let style_path = path.clone();
        stack((
            svg(move || config.get().file_svg(&path).0).style(move |s| {
                let config = config.get();
                let size = config.ui.icon_size() as f32;
                let color = config.file_svg(&style_path).1;
                s.min_width(size)
                    .size(size, size)
                    .margin(6.0)
                    .apply_opt(color, Style::color)
            }),
            label(move || file_name.clone()).style(move |s| {
                let config = config.get();
                let size = config.ui.icon_size() as f32;
                let max_width = panel_width.get() as f32
                    - 10.0
                    - size
                    - 6.0
                    - size
                    - 6.0
                    - 10.0
                    - size
                    - 6.0;
                s.text_ellipsis()
                    .margin_right(6.0)
                    .max_width(max_width)
                    .selectable(false)
            }),
            label(move || folder.clone()).style(move |s| {
                s.text_ellipsis()
                    .flex_grow(1.0f32)
                    .flex_basis(0.0)
                    .color(config.get().color(LapceColor::EDITOR_DIM))
                    .min_width(0.0)
                    .selectable(false)
            }),
            container({
                svg(move || {
                    let svg = match &diff {
                        FileDiff::Modified(_) => LapceIcons::SCM_DIFF_MODIFIED,
                        FileDiff::Added(_) => LapceIcons::SCM_DIFF_ADDED,
                        FileDiff::Deleted(_) => LapceIcons::SCM_DIFF_REMOVED,
                        FileDiff::Renamed(_, _) => LapceIcons::SCM_DIFF_RENAMED,
                    };
                    config.get().ui_svg(svg)
                })
                .style(move |s| {
                    let config = config.get();
                    let size = config.ui.icon_size() as f32;
                    let color = match &diff_for_style {
                        FileDiff::Modified(_) => LapceColor::SOURCE_CONTROL_MODIFIED,
                        FileDiff::Added(_) => LapceColor::SOURCE_CONTROL_ADDED,
                        FileDiff::Deleted(_) => LapceColor::SOURCE_CONTROL_REMOVED,
                        FileDiff::Renamed(_, _) => {
                            LapceColor::SOURCE_CONTROL_MODIFIED
                        }
                    };
                    let color = config.color(color);
                    s.min_width(size).size(size, size).color(color)
                })
            })
            .style(|s| {
                s.absolute()
                    .size_pct(100.0, 100.0)
                    .padding_right(20.0)
                    .items_center()
                    .justify_end()
            }),
        ))
        .on_click_stop(move |_| {
            internal_command.send(InternalCommand::OpenFileChanges {
                path: path_for_click.clone(),
            });
        })
        .style(move |s| {
            let config = config.get();
            let size = config.ui.icon_size() as f32;
            s.padding_left(10.0)
                .padding_right(10.0 + size + 6.0)
                .width_pct(100.0)
                .items_center()
                .cursor(CursorStyle::Pointer)
                .hover(|s| {
                    s.background(config.color(LapceColor::PANEL_HOVERED_BACKGROUND))
                })
        })
    };

    container({
        scroll({
            dyn_stack(
                move || file_diffs.get(),
                |(path, diff)| (path.to_path_buf(), diff.clone()),
                view_fn,
            )
            .style(|s| s.line_height(1.6).flex_col().width_pct(100.0))
        })
        .style(|s| s.absolute().size_pct(100.0, 100.0))
    })
    .on_resize(move |rect| {
        panel_rect.set(rect);
    })
    .style(|s| s.size_pct(100.0, 100.0))
}
