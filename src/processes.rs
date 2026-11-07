//! The Processes tab: a searchable, sortable, virtualized process table with a
//! right-click menu for ending processes.

use std::ops::Range;

use gpui::{
    AnyElement, App, ClickEvent, ClipboardItem, Context, Div, Entity, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Rgba, ScrollStrategy, SharedString, Stateful,
    UniformListScrollHandle, Window, anchored, canvas, deferred, div, point, prelude::*, px,
    uniform_list,
};

use crate::{
    app::{TaskManager, ToastKind},
    process_control::{SignalError, SignalKind, signal_process},
    process_row::RowCells,
    sampler::ProcessRow,
    text_input::TextInput,
    theme::theme,
    ui::empty_state,
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
    pub const ALL: [Column; 6] = [
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

    pub const NAME_MIN_WIDTH: Pixels = px(120.);

    /// Width of every column except Name, which takes the remaining space.
    pub fn fixed_width(self) -> Option<Pixels> {
        match self {
            Column::Name => None,
            Column::Pid => Some(PID_COL_WIDTH),
            Column::User => Some(USER_COL_WIDTH),
            Column::Cpu | Column::Memory | Column::Disk => Some(NUMERIC_COL_WIDTH),
        }
    }

    /// Sizes a header cell; rows use [`crate::process_row::column_spans`], which
    /// computes the same layout.
    fn size<E: Styled>(self, el: E) -> E {
        match self.fixed_width() {
            Some(width) => el.w(width).flex_shrink_0(),
            None => el.flex_1().min_w(Self::NAME_MIN_WIDTH),
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

/// Sorts `order` (indices into `rows`). Ties fall back to PID so rows don't
/// jitter between refreshes.
pub fn sort_rows(rows: &[ProcessRow], order: &mut [usize], column: Column, dir: SortDirection) {
    order.sort_unstable_by(|&a, &b| {
        let (a, b) = (&rows[a], &rows[b]);
        let primary = match column {
            Column::Name => a.display.name_lower.cmp(&b.display.name_lower),
            Column::Pid => a.pid.cmp(&b.pid),
            Column::User => a.display.user_lower.cmp(&b.display.user_lower),
            Column::Cpu => a.cpu.total_cmp(&b.cpu),
            Column::Memory => a.memory.cmp(&b.memory),
            Column::Disk => a.disk.cmp(&b.disk),
        };
        let primary = match dir {
            SortDirection::Ascending => primary,
            SortDirection::Descending => primary.reverse(),
        };
        primary.then_with(|| a.pid.cmp(&b.pid))
    });
}

/// A search query, normalized once rather than per row.
pub struct Query {
    lower: String,
    numeric: bool,
}

impl Query {
    pub fn new(text: &str) -> Self {
        let lower = text.trim().to_lowercase();
        let numeric = !lower.is_empty() && lower.bytes().all(|b| b.is_ascii_digit());
        Self { lower, numeric }
    }

    /// Case-insensitive match on name or user, or a PID prefix for numeric queries.
    pub fn matches(&self, row: &ProcessRow) -> bool {
        self.lower.is_empty()
            || row.display.name_lower.contains(&self.lower)
            || row.display.user_lower.contains(&self.lower)
            || (self.numeric && row.display.pid.starts_with(&self.lower))
    }
}

pub struct ContextMenu {
    pid: u32,
    start_time: u64,
    name: SharedString,
    user: SharedString,
    position: Point<Pixels>,
}

pub struct ProcessesState {
    /// Indices into the snapshot's processes that match the search, in display order.
    pub rows: Vec<usize>,
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
        let query = Query::new(state.search.read(cx).text());
        state.rows.clear();
        state.rows.extend(
            snapshot
                .processes
                .iter()
                .enumerate()
                .filter(|(_, row)| query.matches(row))
                .map(|(ix, _)| ix),
        );
        sort_rows(
            &snapshot.processes,
            &mut state.rows,
            state.sort_column,
            state.sort_direction,
        );
        // Never leave an invisible process selected: Delete would act on it.
        if state.selected_pid.is_some() && self.selected_row().is_none() {
            self.processes.selected_pid = None;
        }
    }

    fn visible_rows(&self) -> impl Iterator<Item = &ProcessRow> {
        let processes = self.snapshot.as_ref().map_or(&[][..], |s| &s.processes);
        self.processes.rows.iter().map(move |&ix| &processes[ix])
    }

    fn selected_row(&self) -> Option<&ProcessRow> {
        let pid = self.processes.selected_pid?;
        self.visible_rows().find(|r| r.pid == pid)
    }

    fn sort_by(&mut self, column: Column, cx: &mut Context<Self>) {
        let state = &mut self.processes;
        if state.sort_column == column {
            state.sort_direction = state.sort_direction.flipped();
        } else {
            state.sort_column = column;
            state.sort_direction = column.default_direction();
        }
        self.rebuild_rows(cx);
        cx.notify();
    }

    /// Moves the selection by `delta` rows and keeps it in view.
    pub fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.processes.rows.len();
        if len == 0 {
            return;
        }
        let next = match self.processes.selected_pid {
            Some(pid) => self
                .visible_rows()
                .position(|r| r.pid == pid)
                .map_or(0, |ix| ix.saturating_add_signed(delta).min(len - 1)),
            None => 0,
        };
        let next_pid = self.visible_rows().nth(next).map(|r| r.pid);
        self.processes.selected_pid = next_pid;
        self.processes
            .scroll_handle
            .scroll_to_item(next, ScrollStrategy::Nearest);
        cx.notify();
    }

    /// Ends the selected process (SIGTERM), or kills it (SIGKILL) when `force`.
    pub fn signal_selected(&mut self, kind: SignalKind, cx: &mut Context<Self>) {
        let Some(row) = self.selected_row() else {
            return;
        };
        let (pid, start_time, name) = (row.pid, row.start_time, row.name.clone());
        self.send_signal(pid, start_time, &name, kind, cx);
    }

    fn send_signal(
        &mut self,
        pid: u32,
        start_time: u64,
        name: &str,
        kind: SignalKind,
        cx: &mut Context<Self>,
    ) {
        let (message, toast) = match (signal_process(pid, start_time, kind), kind) {
            (Ok(()), SignalKind::Kill) => {
                (format!("Force-killed {name} ({pid})"), ToastKind::Success)
            }
            (Ok(()), SignalKind::Terminate) => {
                (format!("Asked {name} ({pid}) to close"), ToastKind::Success)
            }
            (Err(SignalError::PermissionDenied), _) => (
                format!("Permission denied: {name} ({pid}) belongs to another user"),
                ToastKind::Error,
            ),
            (Err(SignalError::Exited), _) => (
                format!("{name} ({pid}) has already exited"),
                ToastKind::Info,
            ),
            (Err(SignalError::Other(err)), _) => (
                format!("Could not end {name} ({pid}): {err}"),
                ToastKind::Error,
            ),
        };
        self.show_toast(message, toast, cx);
    }

    fn copy_to_clipboard(&mut self, text: String, what: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.show_toast(format!("Copied {what}"), ToastKind::Info, cx);
    }

    fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.processes.context_menu.take().is_some() {
            cx.notify();
        }
    }

    /// Benchmark helper: scrolls the table continuously, bouncing between the ends.
    pub fn bench_scroll(&self, elapsed: f32) {
        let handle = self.processes.scroll_handle.0.borrow();
        let max = (self.processes.rows.len() as f32 * f32::from(ROW_HEIGHT)
            - f32::from(handle.base_handle.bounds().size.height))
        .max(1.0);
        let travel = (elapsed * 2400.0) % (2.0 * max);
        let y = if travel > max {
            2.0 * max - travel
        } else {
            travel
        };
        handle.base_handle.set_offset(point(px(0.), px(-y)));
    }

    pub fn render_processes(&self, cx: &mut Context<Self>) -> AnyElement {
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
                            let this = &*this;
                            let Some(snapshot) = &this.snapshot else {
                                return Vec::new();
                            };
                            let order = this.processes.rows.get(range.clone()).unwrap_or(&[]);
                            order
                                .iter()
                                .zip(range)
                                .map(|(&ix, i)| this.render_row(i, &snapshot.processes[ix], cx))
                                .collect()
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
                column.size(
                    div()
                        .id(("column", column as usize))
                        .flex()
                        .h_full()
                        .items_center()
                        .px_2()
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|s| s.bg(t.row_hover).text_color(t.text))
                        .when(active, |s| s.text_color(t.accent))
                        .when(column.is_numeric(), |s| s.justify_end())
                        .child(column.label())
                        .child(arrow)
                        .on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.sort_by(column, cx)
                            }),
                        ),
                )
            }))
    }

    fn render_row(&self, index: usize, row: &ProcessRow, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = theme();
        let highlighted = self.processes.selected_pid == Some(row.pid)
            || self
                .processes
                .context_menu
                .as_ref()
                .is_some_and(|m| m.pid == row.pid);
        let memory_total = self.snapshot.as_ref().map_or(0, |s| s.memory_total);
        let (pid, start_time) = (row.pid, row.start_time);
        let (name, user) = (row.name.clone(), row.user.clone());

        div()
            .id(("process", pid as usize))
            .w_full()
            .h(ROW_HEIGHT)
            .px_2()
            .text_sm()
            .when(index % 2 == 1, |s| s.bg(t.surface_raised.opacity(0.35)))
            .hover(|s| s.bg(t.row_hover))
            .when(highlighted, |s| s.bg(t.row_selected))
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
                        start_time,
                        name: name.clone(),
                        user: user.clone(),
                        position: ev.position,
                    });
                    window.focus(&this.focus_handle, cx);
                    cx.notify();
                }),
            )
            .child(RowCells::new(row, memory_total))
    }

    pub fn render_context_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let t = theme();
        let menu = self.processes.context_menu.as_ref()?;
        let (pid, start_time, name) = (menu.pid, menu.start_time, menu.name.clone());

        let signal_item = |id, label, shortcut, color, kind, cx: &mut Context<Self>| {
            let name = name.clone();
            menu_item(
                id,
                label,
                shortcut,
                color,
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.close_context_menu(cx);
                    this.send_signal(pid, start_time, &name, kind, cx);
                }),
            )
        };
        let copy_item = |id, label, text: String, what: &'static str, cx: &mut Context<Self>| {
            menu_item(
                id,
                label,
                "",
                t.text,
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.close_context_menu(cx);
                    this.copy_to_clipboard(text.clone(), what, cx);
                }),
            )
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
            .on_mouse_down_out(
                cx.listener(|this, _: &MouseDownEvent, _, cx| this.close_context_menu(cx)),
            )
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
                    .child(div().text_xs().text_color(t.text_faint).child(
                        if menu.user.is_empty() {
                            format!("PID {pid}")
                        } else {
                            format!("PID {pid} · {}", menu.user)
                        },
                    )),
            )
            .child(menu_divider())
            .child(signal_item(
                "end",
                "End process",
                "Del",
                t.text,
                SignalKind::Terminate,
                cx,
            ))
            .child(signal_item(
                "kill",
                "Force kill",
                "Shift+Del",
                t.danger,
                SignalKind::Kill,
                cx,
            ))
            .child(menu_divider())
            .child(copy_item(
                "copy-name",
                "Copy name",
                name.to_string(),
                "process name",
                cx,
            ))
            .child(copy_item(
                "copy-pid",
                "Copy PID",
                pid.to_string(),
                "PID",
                cx,
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

fn menu_item(
    id: &'static str,
    label: &'static str,
    shortcut: &'static str,
    color: Rgba,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, name: &str, user: &str, cpu: f32) -> ProcessRow {
        ProcessRow::new(
            pid,
            0,
            name.to_owned().into(),
            user.to_owned().into(),
            cpu,
            0,
            0,
        )
    }

    fn sorted_pids(rows: &[ProcessRow], column: Column, dir: SortDirection) -> Vec<u32> {
        let mut order: Vec<usize> = (0..rows.len()).collect();
        sort_rows(rows, &mut order, column, dir);
        order.into_iter().map(|ix| rows[ix].pid).collect()
    }

    #[test]
    fn sorts_numeric_descending_with_pid_tiebreak() {
        let rows = [
            row(3, "a", "", 1.0),
            row(1, "b", "", 5.0),
            row(2, "c", "", 1.0),
        ];
        assert_eq!(
            sorted_pids(&rows, Column::Cpu, SortDirection::Descending),
            [1, 2, 3]
        );
    }

    #[test]
    fn sorts_names_case_insensitively() {
        let rows = [
            row(1, "zsh", "", 0.0),
            row(2, "Firefox", "", 0.0),
            row(3, "bash", "", 0.0),
        ];
        assert_eq!(
            sorted_pids(&rows, Column::Name, SortDirection::Ascending),
            [3, 2, 1]
        );
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
        let matches = |q: &str| Query::new(q).matches(&firefox);
        assert!(matches(""));
        assert!(matches("  fire "));
        assert!(matches("TREV"));
        assert!(matches("42"));
        assert!(!matches("24"));
        assert!(!matches("chrome"));
    }
}
