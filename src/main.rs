mod app;
mod bench;
mod format;
mod graph;
mod performance;
mod process_control;
mod process_row;
mod processes;
mod sampler;
mod text_input;
mod theme;
mod ui;

use gpui::{App, Bounds, TitlebarOptions, WindowBounds, WindowOptions, prelude::*, px, size};

use app::TaskManager;

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        app::bind_keys(cx);
        text_input::bind_keys(cx);

        let bounds = Bounds::centered(None, size(px(1080.0), px(740.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Task Manager".into()),
                    ..Default::default()
                }),
                app_id: Some("taskmanager-rs".into()),
                window_min_size: Some(size(px(720.0), px(480.0))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| TaskManager::new(window, cx)),
        )
        .expect("failed to open window");
        cx.activate(true);
    });
}
