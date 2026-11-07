# taskmanager-rs

A fast, good-looking task manager for Linux, built in Rust with [gpui](https://gpui.rs) (Zed's GPU-accelerated UI framework).

![Processes tab](docs/screenshots/processes.png)

<details>
<summary>Performance tab</summary>

![Performance tab](docs/screenshots/performance.png)

</details>

## Features

- **Processes** — live, sortable, virtualized process table (name, PID, user, CPU, memory, disk) with heat-tinted cells for busy processes
- **Search** — filter by name, user, or PID prefix (`Ctrl+F`)
- **End processes** — right-click menu or keyboard: `Del` sends SIGTERM, `Shift+Del` sends SIGKILL
- **Performance** — 60-second graphs for CPU (overall and per core), memory and swap, network (per interface), and disk I/O
- **Network usage** — system-wide receive/send rates (virtual and loopback interfaces excluded)

## Keyboard shortcuts

| Key | Action |
| --- | --- |
| `Ctrl+1` / `Ctrl+2` | Processes / Performance tab |
| `Ctrl+F` | Focus search |
| `↑` / `↓` | Move selection |
| `Del` / `Shift+Del` | End / force-kill selected process |
| `Esc` | Close menu, clear search |
| `Ctrl+Q` | Quit |

## Building

Requires a recent stable Rust toolchain and the usual gpui Linux dependencies (Vulkan, Wayland and/or X11, xkbcommon, fontconfig).

```sh
cargo run --release
```

gpui is pulled from the Zed repository at a pinned commit (the crates.io release is outdated), so the first build downloads the Zed repo and takes a few minutes. Dependencies are optimized even in debug builds, so plain `cargo run` stays smooth while the app itself remains quick to recompile.

## Performance

The UI is built to hold the display's full refresh rate (tested at 165 Hz) while
scrolling and hovering:

- Sampling and all text formatting happen on a background thread; the UI thread only draws.
- Table rows are a single custom-painted element instead of nested layout boxes.
- Graph paths are cached and rebuilt only when their data or size changes.

To measure it on your machine, run the built-in frame benchmark with the window visible:

```sh
TASKMANAGER_BENCH=1 cargo run --release
```

It cycles through every view (including fast scrolling) and prints frame rate,
frame-interval and per-frame CPU percentiles, plus dropped frames, for each.

## Notes

- Per-process network usage isn't shown: Linux only exposes it to privileged tools.
- `src/text_input.rs` is adapted from gpui's `examples/input.rs` (Zed Industries, Apache-2.0).

## License

MIT — see [LICENSE](LICENSE). `src/text_input.rs` is derived from Apache-2.0 code from the Zed project; its notice is kept in that file.
