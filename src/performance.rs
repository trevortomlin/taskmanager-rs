//! The Performance tab: live graphs for CPU, memory, network and disk, with a
//! sidebar of sparklines to pick which resource to inspect.

use gpui::{
    AnyElement, ClickEvent, Context, Div, Rgba, SharedString, Stateful, div, prelude::*, px,
};

use crate::{
    app::TaskManager,
    format,
    graph::{HISTORY_LABEL, Line, Series, graph},
    sampler::Snapshot,
    theme::theme,
    ui::{color_dot, empty_state, section_heading, stat_tile},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Resource {
    Cpu,
    Memory,
    Network,
    Disk,
}

impl Resource {
    pub const ALL: [Resource; 4] = [
        Resource::Cpu,
        Resource::Memory,
        Resource::Network,
        Resource::Disk,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Resource::Cpu => "CPU",
            Resource::Memory => "Memory",
            Resource::Network => "Network",
            Resource::Disk => "Disk",
        }
    }

    pub fn color(self) -> Rgba {
        let t = theme();
        match self {
            Resource::Cpu => t.cpu,
            Resource::Memory => t.memory,
            Resource::Network => t.net_rx,
            Resource::Disk => t.disk,
        }
    }

    /// The lines graphed for this resource, plus the value they're scaled to.
    pub fn graph_lines(self, h: &History) -> (Vec<Line>, f32) {
        let t = theme();
        match self {
            Resource::Cpu => (vec![Line::new(&h.cpu, t.cpu)], 100.0),
            Resource::Memory => (
                vec![
                    Line::new(&h.memory, t.memory),
                    Line::new(&h.swap, t.swap).without_area(),
                ],
                100.0,
            ),
            Resource::Network => (
                vec![
                    Line::new(&h.net_rx, t.net_rx),
                    Line::new(&h.net_tx, t.net_tx).without_area(),
                ],
                h.network_ceiling(),
            ),
            Resource::Disk => (
                vec![
                    Line::new(&h.disk_read, t.disk),
                    Line::new(&h.disk_write, t.disk_write).without_area(),
                ],
                h.disk_ceiling(),
            ),
        }
    }
}

/// Rolling history of everything the Performance tab graphs.
#[derive(Default)]
pub struct History {
    pub cpu: Series,
    pub cores: Vec<Series>,
    /// Memory and swap as percentages of their totals.
    pub memory: Series,
    pub swap: Series,
    pub net_rx: Series,
    pub net_tx: Series,
    pub disk_read: Series,
    pub disk_write: Series,
}

impl History {
    pub fn record(&mut self, s: &Snapshot) {
        self.cpu.push(s.cpu);
        self.cores.resize_with(s.cpu_cores.len(), Series::default);
        for (series, usage) in self.cores.iter_mut().zip(&s.cpu_cores) {
            series.push(*usage);
        }
        self.memory.push(s.memory_fraction() * 100.0);
        self.swap.push(s.swap_fraction() * 100.0);
        self.net_rx.push(s.net_rx as f32);
        self.net_tx.push(s.net_tx as f32);
        self.disk_read.push(s.disk_read as f32);
        self.disk_write.push(s.disk_write as f32);
    }

    pub fn network_ceiling(&self) -> f32 {
        format::nice_ceiling(self.net_rx.max().max(self.net_tx.max()))
    }

    fn disk_ceiling(&self) -> f32 {
        format::nice_ceiling(self.disk_read.max().max(self.disk_write.max()))
    }
}

/// Everything that differs between the resource detail views.
struct Detail {
    subtitle: SharedString,
    scale_label: String,
    legend: Vec<(&'static str, Rgba)>,
    stats: Vec<(&'static str, String)>,
    extra: Option<AnyElement>,
}

impl TaskManager {
    pub fn render_performance(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(snapshot) = &self.snapshot else {
            return empty_state("Gathering system information…").into_any_element();
        };

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_row()
            .gap_3()
            .mx_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .w(px(230.))
                    .flex_shrink_0()
                    .children(
                        Resource::ALL
                            .map(|resource| self.render_resource_card(resource, snapshot, cx)),
                    ),
            )
            .child(self.render_detail(snapshot))
            .into_any_element()
    }

    fn render_resource_card(
        &self,
        resource: Resource,
        s: &Snapshot,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = theme();
        let selected = self.resource == resource;
        let (value, detail) = match resource {
            Resource::Cpu => (format::percent(s.cpu), format::frequency(s.cpu_frequency)),
            Resource::Memory => (
                format::bytes(s.memory_used),
                format!(
                    "{:.0}% of {}",
                    s.memory_fraction() * 100.0,
                    format::bytes(s.memory_total)
                ),
            ),
            Resource::Network => (
                format!("↓ {}", format::speed(s.net_rx)),
                format!("↑ {}", format::speed(s.net_tx)),
            ),
            Resource::Disk => (
                format!("R {}", format::speed(s.disk_read)),
                format!("W {}", format::speed(s.disk_write)),
            ),
        };
        let (lines, max) = resource.graph_lines(&self.history);

        div()
            .id(("resource", resource as usize))
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .rounded_lg()
            .border_1()
            .cursor_pointer()
            .border_color(if selected {
                resource.color().opacity(0.6)
            } else {
                t.border
            })
            .bg(if selected {
                t.surface_raised
            } else {
                t.surface
            })
            .hover(|s| s.bg(t.surface_raised))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.resource = resource;
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(t.text)
                            .child(color_dot(resource.color()))
                            .child(resource.label()),
                    )
                    .child(div().text_sm().text_color(t.text).child(value)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(t.text_faint)
                    .truncate()
                    .child(detail),
            )
            .child(
                graph(("sidebar", resource as usize), lines, max, 0)
                    .h(px(34.))
                    .w_full()
                    .mt_1(),
            )
    }

    fn detail(&self, s: &Snapshot) -> Detail {
        let t = theme();
        let h = &self.history;
        match self.resource {
            Resource::Cpu => Detail {
                subtitle: s.cpu_brand.clone(),
                scale_label: "100%".into(),
                legend: vec![],
                stats: vec![
                    ("Utilization", format::percent(s.cpu)),
                    ("Speed", format::frequency(s.cpu_frequency)),
                    ("Logical cores", s.cpu_cores.len().to_string()),
                    ("Processes", s.processes.len().to_string()),
                    ("Uptime", format::duration(s.uptime_secs)),
                ],
                extra: Some(self.render_core_grid(s).into_any_element()),
            },
            Resource::Memory => Detail {
                subtitle: format!("{} total", format::bytes(s.memory_total)).into(),
                scale_label: "100%".into(),
                legend: vec![("Memory", t.memory), ("Swap", t.swap)],
                stats: vec![
                    ("In use", format::bytes(s.memory_used)),
                    (
                        "Available",
                        format::bytes(s.memory_total.saturating_sub(s.memory_used)),
                    ),
                    ("Usage", format!("{:.0}%", s.memory_fraction() * 100.0)),
                    (
                        "Swap",
                        format!(
                            "{} / {}",
                            format::bytes(s.swap_used),
                            format::bytes(s.swap_total)
                        ),
                    ),
                ],
                extra: None,
            },
            Resource::Network => Detail {
                subtitle: match s.interfaces.len() {
                    0 => "No active interfaces".into(),
                    1 => s.interfaces[0].name.clone(),
                    n => format!("{n} interfaces").into(),
                },
                scale_label: format::speed(h.network_ceiling() as u64),
                legend: vec![("Receive", t.net_rx), ("Send", t.net_tx)],
                stats: vec![
                    ("Receiving", format::speed(s.net_rx)),
                    ("Sending", format::speed(s.net_tx)),
                    (
                        "Received since boot",
                        format::bytes(s.interfaces.iter().map(|i| i.total_rx).sum()),
                    ),
                    (
                        "Sent since boot",
                        format::bytes(s.interfaces.iter().map(|i| i.total_tx).sum()),
                    ),
                ],
                extra: Some(render_interfaces(s).into_any_element()),
            },
            Resource::Disk => Detail {
                subtitle: "All processes".into(),
                scale_label: format::speed(h.disk_ceiling() as u64),
                legend: vec![("Read", t.disk), ("Write", t.disk_write)],
                stats: vec![
                    ("Reading", format::speed(s.disk_read)),
                    ("Writing", format::speed(s.disk_write)),
                ],
                extra: None,
            },
        }
    }

    fn render_detail(&self, s: &Snapshot) -> impl IntoElement {
        let t = theme();
        let Detail {
            subtitle,
            scale_label,
            legend,
            stats,
            extra,
        } = self.detail(s);
        let (lines, max) = self.resource.graph_lines(&self.history);

        div()
            .id("performance-detail")
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .rounded_lg()
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_baseline()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(t.text)
                            .child(self.resource.label()),
                    )
                    .child(div().text_sm().text_color(t.text_muted).child(subtitle)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_xs()
                            .text_color(t.text_faint)
                            .child(div().flex().gap_3().children(legend.into_iter().map(
                                |(label, color)| {
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .child(div().w(px(10.)).h(px(2.)).bg(color))
                                        .child(label)
                                },
                            )))
                            .child(scale_label),
                    )
                    .child(
                        div()
                            .h(px(200.))
                            .flex_shrink_0()
                            .rounded_md()
                            .border_1()
                            .border_color(t.border)
                            .bg(t.background)
                            .overflow_hidden()
                            .child(
                                graph(("detail", self.resource as usize), lines, max, 4)
                                    .size_full(),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_xs()
                            .text_color(t.text_faint)
                            .child(HISTORY_LABEL)
                            .child("now"),
                    ),
            )
            .child(
                div().flex().flex_wrap().gap_3().children(
                    stats
                        .into_iter()
                        .map(|(label, value)| stat_tile(label, value)),
                ),
            )
            .children(extra)
    }

    fn render_core_grid(&self, s: &Snapshot) -> impl IntoElement {
        let t = theme();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(section_heading("Logical processors"))
            .child(div().flex().flex_wrap().gap_2().children(
                self.history.cores.iter().enumerate().map(|(i, series)| {
                    let usage = s.cpu_cores.get(i).copied().unwrap_or(0.0);
                    div()
                        .w(px(132.))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_2()
                        .rounded_md()
                        .border_1()
                        .border_color(t.border)
                        .bg(t.background)
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .text_xs()
                                .child(div().text_color(t.text_faint).child(format!("CPU {i}")))
                                .child(
                                    div().text_color(t.text_muted).child(format::percent(usage)),
                                ),
                        )
                        .child(
                            graph(("core", i), vec![Line::new(series, t.cpu)], 100.0, 0)
                                .h(px(30.))
                                .w_full(),
                        )
                }),
            ))
    }
}

fn render_interfaces(s: &Snapshot) -> impl IntoElement {
    let t = theme();
    let cell = |w: f32| div().w(px(w)).flex().justify_end().flex_shrink_0();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(section_heading("Interfaces"))
        .child(
            div()
                .flex()
                .flex_col()
                .rounded_md()
                .border_1()
                .border_color(t.border)
                .overflow_hidden()
                .text_sm()
                .child(
                    div()
                        .flex()
                        .px_3()
                        .py_1p5()
                        .bg(t.surface_raised)
                        .text_xs()
                        .text_color(t.text_muted)
                        .child(div().flex_1().child("Name"))
                        .child(cell(100.).child("Receive"))
                        .child(cell(100.).child("Send"))
                        .child(cell(110.).child("Total ↓ / ↑")),
                )
                .children(s.interfaces.iter().map(|i| {
                    div()
                        .flex()
                        .px_3()
                        .py_1p5()
                        .border_t_1()
                        .border_color(t.border)
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .text_color(t.text)
                                .child(i.name.clone()),
                        )
                        .child(cell(100.).text_color(t.net_rx).child(format::rate(i.rx)))
                        .child(cell(100.).text_color(t.net_tx).child(format::rate(i.tx)))
                        .child(cell(110.).text_color(t.text_faint).text_xs().child(format!(
                            "{} / {}",
                            format::bytes(i.total_rx),
                            format::bytes(i.total_tx)
                        )))
                })),
        )
}
