//! Color palette (a soft dark theme). Centralized so a light theme can be added later.

use gpui::{Hsla, Rgba};

pub struct Theme {
    pub background: Rgba,
    pub surface: Rgba,
    pub surface_raised: Rgba,
    pub border: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    pub text_faint: Rgba,
    pub row_hover: Rgba,
    pub row_selected: Rgba,
    pub accent: Rgba,
    pub cpu: Rgba,
    pub memory: Rgba,
    pub disk: Rgba,
    pub disk_write: Rgba,
    pub net_rx: Rgba,
    pub net_tx: Rgba,
    pub swap: Rgba,
    pub danger: Rgba,
    pub success: Rgba,
    pub grid: Rgba,
}

pub const DARK: Theme = Theme {
    background: rgb_const(0x11111b),
    surface: rgb_const(0x181825),
    surface_raised: rgb_const(0x1e1e2e),
    border: rgb_const(0x313244),
    text: rgb_const(0xcdd6f4),
    text_muted: rgb_const(0xa6adc8),
    text_faint: rgb_const(0x6c7086),
    row_hover: rgb_const(0x232336),
    row_selected: rgb_const(0x2c2f4a),
    accent: rgb_const(0x89b4fa),
    cpu: rgb_const(0x89b4fa),
    memory: rgb_const(0xcba6f7),
    disk: rgb_const(0xa6e3a1),
    disk_write: rgb_const(0xf9e2af),
    net_rx: rgb_const(0x94e2d5),
    net_tx: rgb_const(0xfab387),
    swap: rgb_const(0xf5c2e7),
    danger: rgb_const(0xf38ba8),
    success: rgb_const(0xa6e3a1),
    grid: rgb_const(0x262637),
};

pub fn theme() -> &'static Theme {
    &DARK
}

/// A translucent tint of `color` whose strength follows `level` (0.0–1.0).
/// Used to "heat" busy cells so heavy processes stand out at a glance.
pub fn heat(color: Rgba, level: f32) -> Hsla {
    let level = level.clamp(0.0, 1.0);
    Hsla::from(color).opacity(if level <= 0.0 {
        0.0
    } else {
        0.08 + level * 0.32
    })
}

const fn rgb_const(hex: u32) -> Rgba {
    Rgba {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rgb_const_matches_gpui_rgb() {
        assert_eq!(gpui::rgb(0x89b4fa), super::rgb_const(0x89b4fa));
    }
}
