//! The Processes tab: a searchable, sortable, virtualized process table with a
//! right-click menu for ending processes.

use std::{cmp::Ordering, ops::Range};

use gpui::{
    AnyElement, App, ClickEvent, ClipboardItem, Context, Div, Entity, Hsla, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollStrategy, SharedString,
    Stateful, UniformListScrollHandle, Window, anchored, canvas, deferred, div, point, prelude::*,
    px, uniform_list,
};
use rustix::{io::Errno, process::Signal};

use crate::{
    app::{TaskManager, ToastKind},
    format,
    sampler::ProcessRow,
    text_input::TextInput,
    theme::{heat, theme},
};

const ROW_HEIGHT: Pixels = px(30.);
const NUMERIC_COL_WIDTH: Pixels = px(96.);
const PID_COL_WIDTH: Pixels = px(72.);
const USER_COL_WIDTH: Pixels = px(110.);
const SCROLLBAR_WIDTH: Pixels = px(6.);
const SCROLLBAR_MIN_THUMB: Pixels = px(28.);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Column {
    Name,
    Pid,
    User,
    Cpu,
    Memory,
    Disk,
}

impl Column {
    const ALL: [Column; 6] = [
        Column::Name,
        Column::Pid,
        Column::User,
        Column::Cpu,
        Column::Memory,
        Column::Disk,
    ];

    fn label(self) -> &'static str {
        match self {
            Column::Name => "Name",
            Column::Pid => "PID",
            Column::User => "User",
            Column::Cpu => "CPU",
            Column::Memory => "Memory",
            Column::Disk => "Disk",
        }
    }

    /// Numbers are most useful biggest-first; text reads best A→Z.
    fn default_direction(self) -> SortDirection {
        match self {
            Column::Name | Column::User | Column::Pid => SortDirection::Ascending,
            Column::Cpu | Column::Memory | Column::Disk => SortDirection::Descending,
        }
    }

    fn is_numeric(self) -> bool {
        !matches!(self, Column::Name | Column::User)
    }

    fn width(self) -> Option<Pixels> {
        match self {
            Column::Name => None,
            Column::Pid => Some(PID_COL_WIDTH),
            Column::User => Some(USER_COL_WIDTH),
            Column::Cpu | Column::Memory | Column::Disk => Some(NUMERIC_COL_WIDTH),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortDirection {
    Ascending,
    Descending,
}

impl SortDirection {
    fn flipped(self) -> Self {
        match self {
            SortDirection::Ascending => SortDirection::Descending,
            SortDirection::Descending => SortDirection::Ascending,
        }
    }
}

/// Sorts rows in place. Ties fall back to PID so rows don't jitter between refreshes.
pub fn sort_rows(rows: &mut [ProcessRow], column: Column, direction: SortDirection) {
    rows.sort_by(|a, b| {
        let primary = match column {
            Column::Name => cmp_text(&a.name, &b.name),
            Column::Pid => a.pid.cmp(&b.pid),
            Column::User => cmp_text(&a.user, &b.user),
            Column::Cpu => a.cpu.total_cmp(&b.cpu),
            Column::Memory => a.memory.cmp(&b.memory),
            Column::Disk => a.disk.cmp(&b.disk),
        };
        let primary = match direction {
            SortDirection::Ascending => primary,
            SortDirection::Descending => primary.reverse(),
        };
        primary.then_with(|| a.pid.cmp(&b.pid))
    });
}

fn cmp_text(a: &str, b: &str) -> Ordering {
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
}

/// Case-insensitive match on name or user, or a PID prefix for numeric queries.
pub fn matches_query(row: &ProcessRow, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    let query = query.to_lowercase();
    row.name.to_lowercase().contains(&query)
        || row.user.to_lowercase().contains(&query)
        || (query.bytes().all(|b| b.is_ascii_digit()) && row.pid.to_string().starts_with(&query))
}

pub struct ContextMenu {
    pid: u32,
    name: SharedString,
    position: Point<Pixels>,
}

pub struct ProcessesState {
    /// Processes from the latest snapshot that match the search, sorted for display.
    pub rows: Vec<ProcessRow>,
    sort_column: Column,
    sort_direction: SortDirection,
    pub search: Entity<TextInput>,
    pub selected_pid: Option<u32>,
    scroll_handle: UniformListScrollHandle,
    /// Offset from the scrollbar thumb's top where a drag started.
    scrollbar_drag: Option<Pixels>,
    pub context_menu: Option<ContextMenu>,
}

impl ProcessesState {
    pub fn new(cx: &mut App) -> Self {
        Self {
            rows: Vec::new(),
            sort_column: Column::Cpu,
            sort_direction: Column::Cpu.default_direction(),
            search: cx.new(|cx| TextInput::new("Search by name, user or PID", cx)),
            selected_pid: None,
            scroll_handle: UniformListScrollHandle::new(),
            scrollbar_drag: None,
            context_menu: None,
        }
    }
}

impl TaskManager {
    /// Re-applies the search filter and sort order to the latest snapshot.
    pub fn rebuild_rows(&mut self, cx: &App) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let state = &mut self.processes;
        let query = state.search.read(cx).text().to_owned();
        state.rows = snapshot
            .processes
            .iter()
            .filter(|row| matches_query(row, &query))
            .cloned()
            .collect();
        sort_rows(&mut state.rows, state.sort_column, state.sort_direction);
        // Never leave an invisible process selected: Delete would act on it.
        if let Some(pid) = state.selected_pid
            && !state.rows.iter().any(|r| r.pid == pid)
        {
            state.selected_pid = None;
        }
    }

    fn sort_by(&mut self, column: Column, cx: &mut Context<Self>) {
        let state = &mut self.processes;
        if state.sort_column == column {
            state.sort_direction = state.sort_direction.flipped();
        } else {
            state.sort_column = column;
            state.sort_direction = column.default_direction();
        }
        sort_rows(&mut state.rows, state.sort_column, state.sort_direction);
        cx.notify();
    }

    /// Moves the selection by `delta` rows and keeps it in view.
    pub fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let state = &mut self.processes;
        if state.rows.is_empty() {
            return;
        }
        let current = state
            .selected_pid
            .and_then(|pid| state.rows.iter().position(|r| r.pid == pid));
        let next = match current {
            Some(ix) => ix.saturating_add_signed(delta).min(state.rows.len() - 1),
            None => 0,
        };
        state.selected_pid = Some(state.rows[next].pid);
        state
            .scroll_handle
            .scroll_to_item(next, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn selected_row(&self) -> Option<&ProcessRow> {
        let pid = self.processes.selected_pid?;
        self.processes.rows.iter().find(|r| r.pid == pid)
    }

    /// Sends SIGTERM (or SIGKILL when `force`) to the selected process.
    pub fn signal_selected(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(row) = self.selected_row() else {
            return;
        };
        let (pid, name) = (row.pid, row.name.clone());
        self.signal_process(pid, name, force, cx);
    }

    fn signal_process(
        &mut self,
        pid: u32,
        name: SharedString,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        let signal = if force { Signal::KILL } else { Signal::TERM };
        let result = i32::try_from(pid)
            .ok()
            .and_then(rustix::process::Pid::from_raw)
            .ok_or(Errno::INVAL)
            .and_then(|pid| rustix::process::kill_process(pid, signal));
        let (message, kind) = match result {
            Ok(()) if force => (format!("Force-killed {name} ({pid})"), ToastKind::Success),
            Ok(()) => (format!("Asked {name} ({pid}) to close"), ToastKind::Success),
            Err(Errno::PERM) => (
                format!("Permission denied: {name} ({pid}) belongs to another user"),
                ToastKind::Error,
            ),
            Err(Errno::SRCH) => (
                format!("{name} ({pid}) has already exited"),
                ToastKind::Info,
            ),
            Err(err) => (
                format!("Could not end {name} ({pid}): {err}"),
                ToastKind::Error,
            ),
        };
        self.show_toast(message, kind, cx);
    }

    fn copy_to_clipboard(&mut self, text: String, what: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.show_toast(format!("Copied {what}"), ToastKind::Info, cx);
    }

    pub fn render_processes(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme();
        let body = if self.snapshot.is_none() {
            empty_state("Loading processes…").into_any_element()
        } else if self.processes.rows.is_empty() {
            let query = self.processes.search.read(cx).text().trim().to_owned();
            empty_state(format!("No processes match “{query}”")).into_any_element()
        } else {
            div()
                .relative()
                .flex_1()
                .child(
                    uniform_list(
                        "processes",
                        self.processes.rows.len(),
                        cx.processor(|this, range: Range<usize>, _window, cx| {
                            let mut items = Vec::with_capacity(range.len());
                            for i in range {
                                if let Some(row) = this.processes.rows.get(i).cloned() {
                                    items.push(this.render_row(i, &row, cx));
                                }
                            }
                            items
                        }),
                    )
                    .size_full()
                    .track_scroll(&self.processes.scroll_handle),
                )
                .child(self.render_scrollbar(cx))
                .into_any_element()
        };

        div()
            .flex_1()
            .flex()
            .flex_col()
            .mx_4()
            .min_h_0()
            .overflow_hidden()
            .rounded_lg()
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .child(self.render_column_headers(cx))
            .child(body)
            .into_any_element()
    }

    fn render_column_headers(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let state = &self.processes;
        div()
            .flex()
            .flex_row()
            .flex_shrink_0()
            .h(px(34.))
            .px_2()
            .items_center()
            .bg(t.surface_raised)
            .border_b_1()
            .border_color(t.border)
            .text_xs()
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(t.text_muted)
            .children(Column::ALL.map(|column| {
                let active = column == state.sort_column;
                let arrow = match (active, state.sort_direction) {
                    (false, _) => "",
                    (true, SortDirection::Ascending) => " ▲",
                    (true, SortDirection::Descending) => " ▼",
                };
                let cell = div()
                    .id(SharedString::from(format!("col-{}", column.label())))
                    .flex()
                    .h_full()
                    .items_center()
                    .px_2()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(t.row_hover).text_color(t.text))
                    .when(active, |s| s.text_color(t.accent))
                    .when(column.is_numeric(), |s| s.justify_end())
                    .child(format!("{}{arrow}", column.label()))
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.sort_by(column, cx)),
                    );
                size_column(cell, column)
            }))
    }

    fn render_row(&self, index: usize, row: &ProcessRow, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = theme();
        let selected = self.processes.selected_pid == Some(row.pid);
        let menu_open = self
            .processes
            .context_menu
            .as_ref()
            .is_some_and(|m| m.pid == row.pid);
        let mem_total = self.snapshot.as_ref().map_or(0, |s| s.memory_total).max(1);
        let pid = row.pid;
        let name = row.name.clone();

        let numeric = |column: Column, text: String, tint: Hsla| {
            size_column(
                div()
                    .flex()
                    .h_full()
                    .items_center()
                    .justify_end()
                    .px_2()
                    .bg(tint)
                    .child(text),
                column,
            )
        };

        div()
            .id(("process", pid as usize))
            .flex()
            .w_full()
            .flex_row()
            .items_center()
            .h(ROW_HEIGHT)
            .px_2()
            .text_sm()
            .text_color(t.text)
            .when(index % 2 == 1, |s| s.bg(t.surface_raised.opacity(0.35)))
            .hover(|s| s.bg(t.row_hover))
            .when(selected || menu_open, |s| s.bg(t.row_selected))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.processes.selected_pid = Some(pid);
                window.focus(&this.focus_handle, cx);
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    this.processes.selected_pid = Some(pid);
                    this.processes.context_menu = Some(ContextMenu {
                        pid,
                        name: name.clone(),
                        position: ev.position,
                    });
                    window.focus(&this.focus_handle, cx);
                    cx.notify();
                }),
            )
            .child(size_column(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .overflow_hidden()
                    .child(process_badge(&row.name))
                    .child(div().truncate().child(row.name.clone())),
                Column::Name,
            ))
            .child(size_column(
                div()
                    .flex()
                    .justify_end()
                    .px_2()
                    .text_color(t.text_faint)
                    .child(row.pid.to_string()),
                Column::Pid,
            ))
            .child(size_column(
                div()
                    .px_2()
                    .truncate()
                    .text_color(t.text_muted)
                    .child(row.user.clone()),
                Column::User,
            ))
            .child(numeric(
                Column::Cpu,
                format::percent(row.cpu),
                heat(t.cpu, row.cpu / 25.0),
            ))
            .child(numeric(
                Column::Memory,
                format::bytes(row.memory),
                heat(t.memory, row.memory as f32 / mem_total as f32 * 4.0),
            ))
            .child(numeric(
                Column::Disk,
                format::rate(row.disk),
                heat(t.disk, row.disk as f32 / (50.0 * 1024.0 * 1024.0)),
            ))
    }

    pub fn render_context_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let t = theme();
        let menu = self.processes.context_menu.as_ref()?;
        let (pid, name) = (menu.pid, menu.name.clone());
        let user = self
            .snapshot
            .as_ref()
            .and_then(|s| s.processes.iter().find(|p| p.pid == pid))
            .map(|p| p.user.clone())
            .unwrap_or_default();

        let close = |this: &mut Self, cx: &mut Context<Self>| {
            this.processes.context_menu = None;
            cx.notify();
        };

        let menu_el = div()
            .id("process-menu")
            .occlude()
            .w(px(230.))
            .p_1()
            .flex()
            .flex_col()
            .rounded_lg()
            .border_1()
            .border_color(t.border)
            .bg(t.surface_raised)
            .shadow_lg()
            .text_sm()
            .on_mouse_down_out(cx.listener(move |this, _: &MouseDownEvent, _, cx| close(this, cx)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .px_2()
                    .pt_1p5()
                    .pb_2()
                    .child(
                        div()
                            .truncate()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(t.text)
                            .child(name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(t.text_faint)
                            .child(if user.is_empty() {
                                format!("PID {pid}")
                            } else {
                                format!("PID {pid} · {user}")
                            }),
                    ),
            )
            .child(menu_divider())
            .child(menu_item(
                "end",
                "End process",
                "Del",
                t.text,
                cx.listener({
                    let name = name.clone();
                    move |this, _: &ClickEvent, _, cx| {
                        close(this, cx);
                        this.signal_process(pid, name.clone(), false, cx);
                    }
                }),
            ))
            .child(menu_item(
                "kill",
                "Force kill",
                "Shift+Del",
                t.danger,
                cx.listener({
                    let name = name.clone();
                    move |this, _: &ClickEvent, _, cx| {
                        close(this, cx);
                        this.signal_process(pid, name.clone(), true, cx);
                    }
                }),
            ))
            .child(menu_divider())
            .child(menu_item(
                "copy-name",
                "Copy name",
                "",
                t.text,
                cx.listener({
                    let name = name.clone();
                    move |this, _: &ClickEvent, _, cx| {
                        close(this, cx);
                        this.copy_to_clipboard(name.to_string(), "process name", cx);
                    }
                }),
            ))
            .child(menu_item(
                "copy-pid",
                "Copy PID",
                "",
                t.text,
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    close(this, cx);
                    this.copy_to_clipboard(pid.to_string(), "PID", cx);
                }),
            ));

        Some(
            deferred(
                anchored()
                    .position(menu.position)
                    .snap_to_window_with_margin(px(8.))
                    .child(menu_el),
            )
            .with_priority(1),
        )
    }

    fn render_scrollbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let state = &self.processes;
        let (viewport, offset_y, content_height) = {
            let inner = state.scroll_handle.0.borrow();
            let content = inner
                .last_item_size
                .map(|s| s.contents.height)
                .unwrap_or_default();
            (
                inner.base_handle.bounds(),
                inner.base_handle.offset().y,
                content,
            )
        };
        let track = viewport.size.height;
        if track <= px(0.) || content_height <= track {
            return div().id("scrollbar");
        }

        let thumb = (track * (track / content_height)).max(SCROLLBAR_MIN_THUMB);
        let max_scroll = content_height - track;
        let progress = (-offset_y / max_scroll).clamp(0., 1.);
        let thumb_top = (track - thumb) * progress;

        let entity = cx.entity();
        let scroll = state.scroll_handle.0.borrow().base_handle.clone();
        let dragging = state.scrollbar_drag.is_some();

        div()
            .id("scrollbar")
            .absolute()
            .top(thumb_top)
            .right(px(3.))
            .w(SCROLLBAR_WIDTH)
            .h(thumb)
            .rounded_full()
            .bg(if dragging { t.text_faint } else { t.border })
            .hover(|s| s.bg(t.text_faint))
            .child(
                canvas(
                    |_, _, _| (),
                    move |thumb_bounds, _, window, _| {
                        window.on_mouse_event({
                            let entity = entity.clone();
                            move |ev: &MouseDownEvent, _, _, cx| {
                                if thumb_bounds.contains(&ev.position) {
                                    entity.update(cx, |this, cx| {
                                        this.processes.scrollbar_drag =
                                            Some(ev.position.y - thumb_bounds.origin.y);
                                        cx.notify();
                                    });
                                }
                            }
                        });
                        window.on_mouse_event({
                            let entity = entity.clone();
                            move |_: &MouseUpEvent, _, _, cx| {
                                entity.update(cx, |this, cx| {
                                    if this.processes.scrollbar_drag.take().is_some() {
                                        cx.notify();
                                    }
                                });
                            }
                        });
                        window.on_mouse_event(move |ev: &MouseMoveEvent, _, _, cx| {
                            if !ev.dragging() {
                                return;
                            }
                            let Some(grab) = entity.read(cx).processes.scrollbar_drag else {
                                return;
                            };
                            let thumb_top = ev.position.y - viewport.origin.y - grab;
                            let progress = (thumb_top / (track - thumb)).clamp(0., 1.);
                            scroll.set_offset(point(px(0.), -(max_scroll * progress)));
                            cx.notify(entity.entity_id());
                        });
                    },
                )
                .size_full(),
            )
    }
}

/// Applies a column's width so header cells and row cells line up.
fn size_column<E: Styled>(el: E, column: Column) -> E {
    match column.width() {
        Some(w) => el.w(w).flex_shrink_0(),
        None => el.flex_1().min_w(px(120.)),
    }
}

fn empty_state(message: impl Into<SharedString>) -> Div {
    let t = theme();
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(t.text_faint)
        .child(message.into())
}

fn menu_item(
    id: &'static str,
    label: &'static str,
    shortcut: &'static str,
    color: gpui::Rgba,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let t = theme();
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_between()
        .px_2()
        .py_1p5()
        .rounded_md()
        .cursor_pointer()
        .text_color(color)
        .hover(|s| s.bg(t.row_selected))
        .child(label)
        .child(div().text_xs().text_color(t.text_faint).child(shortcut))
        .on_click(on_click)
}

fn menu_divider() -> impl IntoElement {
    div().h(px(1.)).mx_1().my_1().bg(theme().border)
}

/// A small colored initial so processes are easy to tell apart while scanning.
fn process_badge(name: &str) -> impl IntoElement {
    let initial = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "•".into());
    let hue = (name
        .bytes()
        .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32))
        % 360) as f32
        / 360.0;
    div()
        .flex()
        .flex_shrink_0()
        .size(px(18.))
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .bg(gpui::hsla(hue, 0.45, 0.35, 1.0))
        .text_color(gpui::hsla(hue, 0.6, 0.88, 1.0))
        .text_xs()
        .font_weight(gpui::FontWeight::BOLD)
        .child(initial)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, name: &str, user: &str, cpu: f32) -> ProcessRow {
        ProcessRow {
            pid,
            name: name.to_owned().into(),
            user: user.to_owned().into(),
            cpu,
            memory: 0,
            disk: 0,
        }
    }

    fn pids(rows: &[ProcessRow]) -> Vec<u32> {
        rows.iter().map(|r| r.pid).collect()
    }

    #[test]
    fn sorts_numeric_descending_with_pid_tiebreak() {
        let mut rows = vec![
            row(3, "a", "", 1.0),
            row(1, "b", "", 5.0),
            row(2, "c", "", 1.0),
        ];
        sort_rows(&mut rows, Column::Cpu, SortDirection::Descending);
        assert_eq!(pids(&rows), [1, 2, 3]);
    }

    #[test]
    fn sorts_names_case_insensitively() {
        let mut rows = vec![
            row(1, "zsh", "", 0.0),
            row(2, "Firefox", "", 0.0),
            row(3, "bash", "", 0.0),
        ];
        sort_rows(&mut rows, Column::Name, SortDirection::Ascending);
        assert_eq!(pids(&rows), [3, 2, 1]);
    }

    #[test]
    fn default_directions_and_flip() {
        assert_eq!(
            Column::Memory.default_direction(),
            SortDirection::Descending
        );
        assert_eq!(Column::Name.default_direction(), SortDirection::Ascending);
        assert_eq!(
            SortDirection::Descending.flipped(),
            SortDirection::Ascending
        );
    }

    #[test]
    fn search_matches_name_user_and_pid_prefix() {
        let firefox = row(4242, "Firefox", "trevor", 0.0);
        assert!(matches_query(&firefox, ""));
        assert!(matches_query(&firefox, "  fire "));
        assert!(matches_query(&firefox, "TREV"));
        assert!(matches_query(&firefox, "42"));
        assert!(!matches_query(&firefox, "24"));
        assert!(!matches_query(&firefox, "chrome"));
    }
}
