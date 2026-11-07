# taskmanager-rs

A fast, good-looking task manager for Linux, built in Rust with [gpui](https://gpui.rs) (Zed's GPU-accelerated UI framework).

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

gpui is pulled from the Zed repository at a pinned commit (the crates.io release is outdated), so the first build downloads the Zed repo and takes a few minutes.

## Notes

- Per-process network usage isn't shown: Linux only exposes it to privileged tools.
- `src/text_input.rs` is adapted from gpui's `examples/input.rs` (Zed Industries, Apache-2.0).
