//! The root view: owns the sampling loop, shared state, keyboard actions, and the
//! window chrome (header, tabs, search box, status bar).

use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, ClickEvent, Context, FocusHandle, Focusable, KeyBinding, Rgba, SharedString,
    Subscription, Task, Window, actions, canvas, div, point, prelude::*, px, relative,
};

use crate::{
    format,
    graph::{Line, graph},
    performance::{History, Resource},
    processes::ProcessesState,
    sampler::{Sampler, Snapshot},
    text_input::TextInputEvent,
    theme::theme,
};

const CONTEXT: &str = "TaskManager";
const REFRESH_INTERVAL: Duration = Duration::from_secs(1);
/// sysinfo needs a short gap between the first two refreshes for valid CPU numbers.
const FIRST_SAMPLE_DELAY: Duration = Duration::from_millis(300);
const TOAST_DURATION: Duration = Duration::from_secs(5);

actions!(
    task_manager,
    [
        SelectNext,
        SelectPrevious,
        EndProcess,
        ForceKillProcess,
        FocusSearch,
        Dismiss,
        ShowProcesses,
        ShowPerformance,
        Quit,
    ]
);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, ctx),
        KeyBinding::new("up", SelectPrevious, ctx),
        KeyBinding::new("delete", EndProcess, ctx),
        KeyBinding::new("shift-delete", ForceKillProcess, ctx),
        KeyBinding::new("ctrl-f", FocusSearch, ctx),
        KeyBinding::new("escape", Dismiss, ctx),
        KeyBinding::new("ctrl-1", ShowProcesses, ctx),
        KeyBinding::new("ctrl-2", ShowPerformance, ctx),
        KeyBinding::new("ctrl-q", Quit, None),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Processes,
    Performance,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

struct Toast {
    message: SharedString,
    kind: ToastKind,
    shown_at: Instant,
}

pub struct TaskManager {
    pub focus_handle: FocusHandle,
    pub tab: Tab,
    pub snapshot: Option<Snapshot>,
    pub history: History,
    pub processes: ProcessesState,
    pub resource: Resource,
    toast: Option<Toast>,
    _subscriptions: Vec<Subscription>,
    _refresh_task: Task<()>,
}

impl TaskManager {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let processes = ProcessesState::new(cx);
        let search_subscription =
            cx.subscribe(&processes.search, |this, _, _: &TextInputEvent, cx| {
                this.rebuild_rows(cx);
                cx.notify();
            });

        let refresh_task = cx.spawn(async move |this, cx| {
            let mut sampler = Sampler::new();
            let mut delay = FIRST_SAMPLE_DELAY;
            loop {
                cx.background_executor().timer(delay).await;
                delay = REFRESH_INTERVAL;
                // Sampling reads /proc for every process; keep it off the UI thread.
                let (returned, snapshot) = cx
                    .background_executor()
                    .spawn(async move {
                        let snapshot = sampler.sample();
                        (sampler, snapshot)
                    })
                    .await;
                sampler = returned;
                if this
                    .update(cx, |this, cx| this.apply_snapshot(snapshot, cx))
                    .is_err()
                {
                    break; // View was dropped.
                }
            }
        });

        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);

        Self {
            focus_handle,
            tab: Tab::Processes,
            snapshot: None,
            history: History::default(),
            processes,
            resource: Resource::Cpu,
            toast: None,
            _subscriptions: vec![search_subscription],
            _refresh_task: refresh_task,
        }
    }

    fn apply_snapshot(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.history.record(&snapshot);
        self.snapshot = Some(snapshot);
        self.rebuild_rows(cx);
        if self
            .toast
            .as_ref()
            .is_some_and(|t| t.shown_at.elapsed() >= TOAST_DURATION)
        {
            self.toast = None;
        }
        cx.notify();
    }

    pub fn show_toast(&mut self, message: String, kind: ToastKind, cx: &mut Context<Self>) {
        self.toast = Some(Toast {
            message: message.into(),
            kind,
            shown_at: Instant::now(),
        });
        cx.notify();
    }

    fn set_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        self.tab = tab;
        self.processes.context_menu = None;
        // Leaving the search box focused on a tab without it would swallow keys.
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn search_focused(&self, window: &Window, cx: &App) -> bool {
        self.processes
            .search
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    }

    // ---- Actions -------------------------------------------------------------

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        if self.tab == Tab::Processes {
            self.move_selection(1, cx);
        }
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        if self.tab == Tab::Processes {
            self.move_selection(-1, cx);
        }
    }

    fn end_process(&mut self, _: &EndProcess, _: &mut Window, cx: &mut Context<Self>) {
        if self.tab == Tab::Processes {
            self.signal_selected(false, cx);
        }
    }

    fn force_kill_process(
        &mut self,
        _: &ForceKillProcess,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Shift+Delete is a text-editing chord too; never kill from inside the search box.
        if self.tab == Tab::Processes && !self.search_focused(window, cx) {
            self.signal_selected(true, cx);
        }
    }

    fn focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.set_tab(Tab::Processes, window, cx);
        window.focus(&self.processes.search.focus_handle(cx), cx);
    }

    /// Escape peels back one layer: menu → search text → search focus.
    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        if self.processes.context_menu.take().is_some() {
            cx.notify();
        } else if !self.processes.search.read(cx).text().is_empty() {
            self.processes
                .search
                .update(cx, |input, cx| input.clear(window, cx));
        } else if self.search_focused(window, cx) {
            window.focus(&self.focus_handle, cx);
        }
    }

    fn show_processes(&mut self, _: &ShowProcesses, window: &mut Window, cx: &mut Context<Self>) {
        self.set_tab(Tab::Processes, window, cx);
    }

    fn show_performance(
        &mut self,
        _: &ShowPerformance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_tab(Tab::Performance, window, cx);
    }

    // ---- Chrome ----------------------------------------------------------------

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let h = &self.history;
        let subtitle: SharedString = match &self.snapshot {
            Some(s) => format!(
                "{} processes · {} logical cores · up {}",
                s.processes.len(),
                s.cpu_cores.len(),
                format::duration(s.uptime_secs)
            )
            .into(),
            None => "Gathering system information…".into(),
        };
        let s = self.snapshot.clone().unwrap_or_default();
        let mem_fraction = if s.memory_total == 0 {
            0.0
        } else {
            s.memory_used as f32 / s.memory_total as f32
        };
        let net_ceiling = format::nice_ceiling(h.net_rx.max().max(h.net_tx.max()));

        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_3()
            .px_5()
            .pt_5()
            .pb_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(220.))
                    .gap_0p5()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(t.text)
                            .child("Task Manager"),
                    )
                    .child(div().text_sm().text_color(t.text_faint).child(subtitle)),
            )
            .child(
                self.stat_card(
                    Resource::Cpu,
                    format::percent(s.cpu),
                    None,
                    graph(vec![Line::new(&h.cpu, t.cpu)], 100.0, 0)
                        .h(px(22.))
                        .w_full()
                        .into_any_element(),
                    cx,
                ),
            )
            .child(self.stat_card(
                Resource::Memory,
                format::bytes(s.memory_used),
                Some(format!("of {}", format::bytes(s.memory_total))),
                progress_bar(mem_fraction, t.memory).into_any_element(),
                cx,
            ))
            .child(
                self.stat_card(
                    Resource::Network,
                    format!("↓ {}", format::speed(s.net_rx)),
                    Some(format!("↑ {}", format::speed(s.net_tx))),
                    graph(
                        vec![
                            Line::new(&h.net_rx, t.net_rx),
                            Line::new(&h.net_tx, t.net_tx).without_area(),
                        ],
                        net_ceiling,
                        0,
                    )
                    .h(px(22.))
                    .w_full()
                    .into_any_element(),
                    cx,
                ),
            )
    }

    /// A summary card; clicking it opens that resource on the Performance tab.
    fn stat_card(
        &self,
        resource: Resource,
        value: String,
        detail: Option<String>,
        visual: AnyElement,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = theme();
        let label = match resource {
            Resource::Cpu => "CPU",
            Resource::Memory => "Memory",
            Resource::Network => "Network",
            Resource::Disk => "Disk",
        };
        let color = match resource {
            Resource::Cpu => t.cpu,
            Resource::Memory => t.memory,
            Resource::Network => t.net_rx,
            Resource::Disk => t.disk,
        };
        div()
            .id(SharedString::from(format!("stat-{label}")))
            .flex()
            .flex_col()
            .justify_between()
            .gap_1p5()
            .w(px(190.))
            .h(px(86.))
            .px_3()
            .py_2p5()
            .rounded_lg()
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .cursor_pointer()
            .hover(|s| s.bg(t.surface_raised))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.resource = resource;
                this.set_tab(Tab::Performance, window, cx);
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_xs()
                    .text_color(t.text_muted)
                    .child(div().size_2().rounded_full().bg(color))
                    .child(label),
            )
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_1p5()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_base()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(t.text)
                            .child(value),
                    )
                    .children(
                        detail
                            .map(|d| div().truncate().text_xs().text_color(t.text_faint).child(d)),
                    ),
            )
            .child(visual)
    }

    fn render_toolbar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let tab_button =
            |tab: Tab, label: &'static str, shortcut: &'static str, cx: &mut Context<Self>| {
                let active = self.tab == tab;
                div()
                    .id(label)
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .text_sm()
                    .when(active, |s| {
                        s.bg(t.row_selected)
                            .text_color(t.text)
                            .font_weight(gpui::FontWeight::MEDIUM)
                    })
                    .when(!active, |s| {
                        s.text_color(t.text_muted).hover(|s| s.text_color(t.text))
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.set_tab(tab, window, cx)
                    }))
                    .child(label)
                    .child(div().text_xs().text_color(t.text_faint).child(shortcut))
            };

        div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_3()
            .px_4()
            .pb_3()
            .child(
                div()
                    .flex()
                    .p_0p5()
                    .gap_0p5()
                    .rounded_lg()
                    .border_1()
                    .border_color(t.border)
                    .bg(t.surface)
                    .child(tab_button(Tab::Processes, "Processes", "Ctrl+1", cx))
                    .child(tab_button(Tab::Performance, "Performance", "Ctrl+2", cx)),
            )
            .when(self.tab == Tab::Processes, |el| {
                el.child(self.render_search(window, cx))
            })
    }

    fn render_search(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let focused = self.search_focused(window, cx);
        let has_text = !self.processes.search.read(cx).text().is_empty();
        div()
            .flex()
            .items_center()
            .gap_2()
            .w(px(300.))
            .h(px(32.))
            .px_3()
            .rounded_lg()
            .border_1()
            .border_color(if focused {
                t.accent.opacity(0.7)
            } else {
                t.border
            })
            .bg(t.surface)
            .text_sm()
            .text_color(t.text)
            .child(search_icon(if focused { t.accent } else { t.text_faint }))
            .child(self.processes.search.clone())
            .when(has_text, |el| {
                el.child(
                    div()
                        .id("clear-search")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(18.))
                        .rounded_full()
                        .cursor_pointer()
                        .text_xs()
                        .text_color(t.text_faint)
                        .hover(|s| s.bg(t.row_hover).text_color(t.text))
                        .child("✕")
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.processes
                                .search
                                .update(cx, |input, cx| input.clear(window, cx));
                        })),
                )
            })
            .when(!focused && !has_text, |el| {
                el.child(div().text_xs().text_color(t.text_faint).child("Ctrl+F"))
            })
    }

    fn render_footer(&self, cx: &App) -> impl IntoElement {
        let t = theme();
        let left: AnyElement = match &self.toast {
            Some(toast) => {
                let color = match toast.kind {
                    ToastKind::Info => t.text_muted,
                    ToastKind::Success => t.success,
                    ToastKind::Error => t.danger,
                };
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_color(color)
                    .child(div().size_1p5().rounded_full().bg(color))
                    .child(toast.message.clone())
                    .into_any_element()
            }
            None => div()
                .child(match self.tab {
                    Tab::Processes => "Right-click a process for actions · ↑↓ to move · Del to end",
                    Tab::Performance => "Click a resource on the left to inspect it",
                })
                .into_any_element(),
        };

        let right: SharedString = match (&self.snapshot, self.tab) {
            (Some(s), Tab::Processes)
                if !self.processes.search.read(cx).text().trim().is_empty() =>
            {
                format!(
                    "{} of {} processes",
                    self.processes.rows.len(),
                    s.processes.len()
                )
                .into()
            }
            _ => "Updates every second".into(),
        };

        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .gap_4()
            .h(px(34.))
            .px_5()
            .text_xs()
            .text_color(t.text_faint)
            .child(div().min_w_0().truncate().child(left))
            .child(div().flex_shrink_0().child(right))
    }
}

impl Focusable for TaskManager {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TaskManager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let body = match self.tab {
            Tab::Processes => self.render_processes(cx),
            Tab::Performance => self.render_performance(cx),
        };

        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::end_process))
            .on_action(cx.listener(Self::force_kill_process))
            .on_action(cx.listener(Self::focus_search))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::show_processes))
            .on_action(cx.listener(Self::show_performance))
            .size_full()
            .flex()
            .flex_col()
            .bg(t.background)
            .text_color(t.text)
            .child(self.render_header(cx))
            .child(self.render_toolbar(window, cx))
            .child(body)
            .child(self.render_footer(cx))
            .children(self.render_context_menu(cx))
    }
}

fn progress_bar(fraction: f32, color: Rgba) -> impl IntoElement {
    div()
        .h(px(4.))
        .w_full()
        .rounded_full()
        .bg(theme().border)
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(color)
                .w(relative(fraction.clamp(0.0, 1.0))),
        )
}

/// A magnifying glass drawn with paths, so no icon font or asset is needed.
fn search_icon(color: Rgba) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let r = px(4.5);
            let c = point(bounds.left() + px(6.), bounds.top() + px(6.));
            let mut ring = gpui::PathBuilder::stroke(px(1.5));
            ring.move_to(point(c.x + r, c.y));
            ring.arc_to(point(r, r), px(0.), false, true, point(c.x - r, c.y));
            ring.arc_to(point(r, r), px(0.), false, true, point(c.x + r, c.y));
            let handle_start = point(c.x + r * 0.72, c.y + r * 0.72);
            ring.move_to(handle_start);
            ring.line_to(point(handle_start.x + px(3.5), handle_start.y + px(3.5)));
            if let Ok(path) = ring.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(15.))
    .flex_shrink_0()
}
