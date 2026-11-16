//! The SPIDER logo, its opening animation, and the small header of the other screens.
//!
//! The opening, in about 1.8 s: the letters are scanned in from left to right (each one shows
//! a few noise blocks, then lands with a white flash), a line grows out of the middle under
//! them, a band of light sweeps across, and the subtitle types itself out. Any key skips it.

use super::paint::{Canvas, Style, Theme, ink, width};

/// "SPIDER" in the ANSI Shadow font: full blocks for the letters, box lines for their shadow.
pub const LOGO: [&str; 6] = [
    "███████╗██████╗ ██╗██████╗ ███████╗██████╗ ",
    "██╔════╝██╔══██╗██║██╔══██╗██╔════╝██╔══██╗",
    "███████╗██████╔╝██║██║  ██║█████╗  ██████╔╝",
    "╚════██║██╔═══╝ ██║██║  ██║██╔══╝  ██╔══██╗",
    "███████║██║     ██║██████╔╝███████╗██║  ██║",
    "╚══════╝╚═╝     ╚═╝╚═════╝ ╚══════╝╚═╝  ╚═╝",
];

pub const WIDTH: i32 = 43;
pub const HEIGHT: i32 = 6;
/// The logo, the line under it and the subtitle.
pub const HERO_HEIGHT: i32 = HEIGHT + 2;
/// When the opening is over, in seconds.
pub const INTRO: f32 = 1.8;

const GLITCH: [char; 3] = ['░', '▒', '▓'];

/// A cheap, stable hash: the same cell flickers the same way on every run.
fn hash(a: i32, b: i32, c: i32) -> u32 {
    let mut x = (a as u32).wrapping_mul(0x9E37_79B1)
        ^ (b as u32).wrapping_mul(0x85EB_CA77)
        ^ (c as u32).wrapping_mul(0xC2B2_AE3D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^ (x >> 15)
}

fn noise(a: i32, b: i32, c: i32) -> f32 {
    hash(a, b, c) as f32 / u32::MAX as f32
}

/// When a cell of the logo lands, in seconds after the start.
fn lands_at(row: i32, column: i32) -> f32 {
    column as f32 / WIDTH as f32 * 0.62 + row as f32 * 0.035 + noise(row, column, 7) * 0.1
}

/// The colour of a cell at rest: a diagonal gradient, the shadow lines darker than the blocks.
fn colour(theme: &Theme, row: i32, column: i32, ch: char) -> super::paint::Rgb {
    let t = (column as f32 + row as f32 * 0.8) / (WIDTH as f32 + HEIGHT as f32 * 0.8);
    let base = theme.at(t);
    if ch == '█' { base } else { base.shade(0.42) }
}

/// How bright a band of light at column `at` makes a cell.
fn shine(row: i32, column: i32, at: f32) -> f32 {
    let distance = column as f32 + row as f32 * 0.55 - at;
    (-(distance * distance) / 5.0).exp()
}

/// Where the light band is during the opening.
pub fn opening_shine(t: f32) -> Option<f32> {
    sweep(t - 0.95, 0.65)
}

/// The light band that crosses the logo every few seconds on the menu.
pub fn idle_shine(seconds: f32) -> Option<f32> {
    sweep(seconds % 7.0 - 1.5, 0.9)
}

/// A band crossing the logo in `length` seconds, starting at `t` = 0.
fn sweep(t: f32, length: f32) -> Option<f32> {
    (0.0..length)
        .contains(&t)
        .then(|| -8.0 + t / length * (WIDTH as f32 + 16.0))
}

/// Whether the screens should be redrawn quickly (something on the logo is moving).
pub fn moving(t: f32, idle_seconds: f32) -> bool {
    t < INTRO || idle_shine(idle_seconds).is_some()
}

/// Draws the logo with its top-left corner at (`x`, `y`), `t` seconds into the opening
/// (anything past [`INTRO`] is the logo at rest), with an optional light band.
pub fn draw(canvas: &mut Canvas, x: i32, y: i32, theme: &Theme, t: f32, band: Option<f32>) {
    for (row, line) in LOGO.iter().enumerate() {
        let row = row as i32;
        for (column, ch) in line.chars().enumerate() {
            let column = column as i32;
            if ch == ' ' {
                continue;
            }
            let landed = lands_at(row, column);
            let mut shown = ch;
            let mut tint = colour(theme, row, column, ch);
            if t < landed - 0.13 {
                continue;
            } else if t < landed {
                // Noise blocks just before the letter lands, changing 24 times a second.
                let frame = (t * 24.0) as i32;
                shown = GLITCH[(hash(row, column, frame) % 3) as usize];
                tint = theme.at(column as f32 / WIDTH as f32).light(0.15);
            } else {
                // A white flash fading into the colour.
                let flash = 1.0 - ((t - landed) / 0.28).min(1.0);
                tint = tint.light(0.8 * flash * flash);
            }
            if let Some(at) = band {
                tint = tint.light(0.62 * shine(row, column, at));
            }
            canvas.put(x + column, y + row, shown, Style::fg(tint).bold());
        }
    }
}

/// The logo centred on the window from row `top`, the line under it and the subtitle.
pub fn hero(canvas: &mut Canvas, top: i32, theme: &Theme, t: f32, band: Option<f32>) {
    let x = (canvas.w - WIDTH) / 2;
    draw(canvas, x, top, theme, t, band);
    // The line grows out of the middle.
    let grown = ((t - 0.45) / 0.45).clamp(0.0, 1.0);
    let half = ((WIDTH + 6) as f32 / 2.0 * grown * (2.0 - grown)) as i32;
    if half > 0 {
        let middle = canvas.w / 2;
        canvas.rule(middle - half, top + HEIGHT, half * 2, theme, 0.75);
    }
    // The subtitle types itself out.
    let subtitle = subtitle();
    let typed = (((t - 0.85) * 70.0).max(0.0) as usize).min(subtitle.chars().count());
    if typed > 0 {
        let start = (canvas.w - width(&subtitle)) / 2;
        for (index, ch) in subtitle.chars().take(typed).enumerate() {
            let style = if ch == '·' {
                Style::fg(theme.accent())
            } else {
                Style::fg(ink::MUTED)
            };
            canvas.put(start + index as i32, top + HEIGHT + 1, ch, style);
        }
    }
}

pub fn subtitle() -> String {
    format!(
        "Discord Rich Presence  ·  sans token  ·  v{}",
        env!("CARGO_PKG_VERSION")
    )
}

/// The small header of every other screen: the name in the gradient, the screen's title,
/// and a line across the window.
pub fn header(canvas: &mut Canvas, theme: &Theme, title: &str) {
    let end = canvas.gradient(3, 1, "S P I D E R", theme);
    canvas.text(end + 2, 1, "·", Style::fg(ink::FAINT));
    canvas.text(end + 4, 1, title, Style::fg(ink::TEXT).bold());
    let w = canvas.w;
    canvas.rule(0, 2, w, theme, 0.55);
}

/// The key hints on the last row: keys bright, what they do faint.
pub fn footer(canvas: &mut Canvas, hints: &[(&str, &str)]) {
    let text_width: i32 = hints
        .iter()
        .map(|(key, what)| width(key) + 1 + width(what))
        .sum::<i32>()
        + 5 * (hints.len() as i32 - 1).max(0);
    let mut x = (canvas.w - text_width).max(1) / 2;
    let y = canvas.h - 1;
    for (index, (key, what)) in hints.iter().enumerate() {
        if index > 0 {
            x = canvas.text(x, y, "  ·  ", Style::fg(ink::GHOST));
        }
        x = canvas.text(x, y, key, Style::fg(ink::TEXT).bold());
        x = canvas.text(x + 1, y, what, Style::fg(ink::FAINT));
    }
}

#[cfg(test)]
mod tests {
    use super::super::paint::THEMES;
    use super::*;

    #[test]
    fn every_row_of_the_logo_has_the_same_width() {
        for row in LOGO {
            assert_eq!(width(row), WIDTH, "{row}");
        }
    }

    #[test]
    fn the_opening_ends_on_the_whole_logo() {
        let mut canvas = Canvas::new(80, 12);
        hero(&mut canvas, 1, &THEMES[0], INTRO, None);
        let x = (80 - WIDTH) / 2;
        for (row, line) in LOGO.iter().enumerate() {
            let shown: String = (0..WIDTH)
                .map(|column| canvas.at(x + column, 1 + row as i32).unwrap().ch)
                .collect();
            assert_eq!(&shown, line);
        }
        assert!(canvas.plain(1 + HEIGHT + 1).contains("sans token"));
        // Halfway through, part of the logo is still to come.
        let mut early = Canvas::new(80, 12);
        hero(&mut early, 1, &THEMES[0], 0.3, None);
        let drawn = (0..HEIGHT)
            .map(|row| early.plain(1 + row).trim().chars().count())
            .sum::<usize>();
        assert!(drawn > 0 && drawn < 6 * 40);
    }
}
