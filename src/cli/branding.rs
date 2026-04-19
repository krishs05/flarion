use std::io::IsTerminal;

const MARK_SVG: &[u8] = include_bytes!("../../assets/flarion-mark.svg");
const MARK_ASCII: &str = include_str!("../../assets/flarion-mark.ascii.txt");

// Palette — matches brand spec §7.
const EMBER: (u8, u8, u8) = (0xff, 0x6b, 0x35);

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum RenderMode {
    TrueColor,
    Basic16,
    NoColor,
}

pub fn detect_mode() -> RenderMode {
    if std::env::var("NO_COLOR").is_ok() {
        return RenderMode::NoColor;
    }
    if std::env::var("FLARION_ASCII").is_ok() {
        return RenderMode::NoColor;
    }
    if !std::io::stdout().is_terminal() {
        return RenderMode::NoColor;
    }
    match std::env::var("COLORTERM").ok().as_deref() {
        Some("truecolor") | Some("24bit") => RenderMode::TrueColor,
        _ => RenderMode::Basic16,
    }
}

pub fn render_mark(cells_wide: u16, mode: RenderMode) -> String {
    if matches!(mode, RenderMode::NoColor) {
        return MARK_ASCII.to_string();
    }
    let opt = resvg::usvg::Options::default();
    let tree = match resvg::usvg::Tree::from_data(MARK_SVG, &opt) {
        Ok(t) => t,
        Err(_) => return MARK_ASCII.to_string(),
    };
    let view = tree.size();
    let aspect = view.height() / view.width();
    let pixel_w = cells_wide as u32;
    let pixel_h = (pixel_w as f32 * aspect).round() as u32 * 2;
    let Some(mut pixmap) = tiny_skia::Pixmap::new(pixel_w, pixel_h) else {
        return MARK_ASCII.to_string();
    };
    let transform = tiny_skia::Transform::from_scale(
        pixel_w as f32 / view.width(),
        pixel_h as f32 / view.height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let mut out = String::with_capacity((pixel_w * pixel_h * 4) as usize);
    for row in (0..pixel_h).step_by(2) {
        for col in 0..pixel_w {
            let top = sample_alpha(&pixmap, col, row);
            let bot = sample_alpha(&pixmap, col, row + 1);
            let top_c = colorize(top, mode);
            let bot_c = colorize(bot, mode);
            match (top_c, bot_c) {
                (None, None) => out.push(' '),
                (Some(rgb), None) => {
                    push_fg(&mut out, rgb.0, rgb.1, rgb.2);
                    out.push('▀');
                    out.push_str("\x1b[0m");
                }
                (None, Some(rgb)) => {
                    push_bg(&mut out, rgb.0, rgb.1, rgb.2);
                    out.push(' ');
                    out.push_str("\x1b[0m");
                }
                (Some(top_rgb), Some(bot_rgb)) => {
                    push_fg(&mut out, top_rgb.0, top_rgb.1, top_rgb.2);
                    push_bg(&mut out, bot_rgb.0, bot_rgb.1, bot_rgb.2);
                    out.push('▀');
                    out.push_str("\x1b[0m");
                }
            }
        }
        out.push('\n');
    }
    out
}

fn sample_alpha(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> u8 {
    let i = ((y * pm.width() + x) * 4) as usize;
    pm.data().get(i + 3).copied().unwrap_or(0)
}

fn colorize(alpha: u8, mode: RenderMode) -> Option<(u8, u8, u8)> {
    if alpha < 32 {
        return None;
    }
    match mode {
        RenderMode::TrueColor | RenderMode::Basic16 => Some(EMBER),
        RenderMode::NoColor => unreachable!(),
    }
}

fn push_fg(s: &mut String, r: u8, g: u8, b: u8) {
    s.push_str(&format!("\x1b[38;2;{r};{g};{b}m"));
}
fn push_bg(s: &mut String, r: u8, g: u8, b: u8) {
    s.push_str(&format!("\x1b[48;2;{r};{g};{b}m"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_truecolor_produces_nonempty_output() {
        let s = render_mark(16, RenderMode::TrueColor);
        assert!(!s.is_empty());
        assert!(s.contains('\n'));
    }

    #[test]
    fn nocolor_mode_returns_ascii_fallback() {
        let s = render_mark(16, RenderMode::NoColor);
        assert!(!s.is_empty());
        assert!(!s.contains("\x1b["), "ASCII fallback should not contain ANSI escapes");
    }
}
