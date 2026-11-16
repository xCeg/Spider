//! Colours, themes and the character canvas every screen draws on.
//!
//! A screen paints a [`Canvas`] (one [`Cell`] per character, with its own colours), and the
//! terminal sends only the rows that changed: no full clear, so nothing flickers, even in the
//! old console host. Gradients are mixed in OKLab, where a pink-to-blue blend stays bright
//! instead of going muddy through grey.

use std::fmt::Write as _;

/// A 24-bit colour.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const WHITE: Rgb = Rgb(255, 255, 255);
    pub const BLACK: Rgb = Rgb(0, 0, 0);

    /// A straight blend: `t` = 0 keeps `self`, 1 gives `other`.
    pub fn mix(self, other: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let blend = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Rgb(
            blend(self.0, other.0),
            blend(self.1, other.1),
            blend(self.2, other.2),
        )
    }

    /// Darker by `t` (0 = unchanged, 1 = black).
    pub fn shade(self, t: f32) -> Rgb {
        self.mix(Rgb::BLACK, t)
    }

    /// Lighter by `t` (0 = unchanged, 1 = white).
    pub fn light(self, t: f32) -> Rgb {
        self.mix(Rgb::WHITE, t)
    }
}

/// The neutral colours shared by every theme.
pub mod ink {
    use super::Rgb;

    pub const TEXT: Rgb = Rgb(236, 237, 241);
    pub const MUTED: Rgb = Rgb(150, 155, 168);
    pub const FAINT: Rgb = Rgb(88, 92, 104);
    pub const GHOST: Rgb = Rgb(52, 55, 64);
    pub const GOOD: Rgb = Rgb(87, 242, 135);
    pub const WARN: Rgb = Rgb(254, 200, 64);
    pub const BAD: Rgb = Rgb(255, 92, 102);
    /// Discord's own card colours, for the preview.
    pub const CARD: Rgb = Rgb(43, 45, 49);
    pub const CARD_EDGE: Rgb = Rgb(30, 31, 34);
    pub const BUTTON: Rgb = Rgb(78, 80, 88);
}

fn linear(channel: u8) -> f32 {
    let c = f32::from(channel) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn gamma(channel: f32) -> u8 {
    let c = channel.clamp(0.0, 1.0);
    let c = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0).round().clamp(0.0, 255.0) as u8
}

fn to_oklab(colour: Rgb) -> [f32; 3] {
    let (r, g, b) = (linear(colour.0), linear(colour.1), linear(colour.2));
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

fn from_oklab(lab: [f32; 3]) -> Rgb {
    let l = lab[0] + 0.396_337_78 * lab[1] + 0.215_803_76 * lab[2];
    let m = lab[0] - 0.105_561_346 * lab[1] - 0.063_854_17 * lab[2];
    let s = lab[0] - 0.089_484_18 * lab[1] - 1.291_485_5 * lab[2];
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    Rgb(
        gamma(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s),
        gamma(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s),
        gamma(-0.004_196_086 * l - 0.703_418_6 * m + 1.707_614_7 * s),
    )
}

/// A blend through OKLab: even steps for the eye, no grey middle.
pub fn blend(a: Rgb, b: Rgb, t: f32) -> Rgb {
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let (x, y) = (to_oklab(a), to_oklab(b));
    from_oklab([
        x[0] + (y[0] - x[0]) * t,
        x[1] + (y[1] - x[1]) * t,
        x[2] + (y[2] - x[2]) * t,
    ])
}

/// A colour scheme: the gradient of the logo and the accents.
#[derive(Debug)]
pub struct Theme {
    /// `spider.json`'s word.
    pub word: &'static str,
    /// The name the window shows.
    pub name: &'static str,
    /// The gradient, left to right.
    pub stops: [Rgb; 3],
}

pub const THEMES: [Theme; 6] = [
    Theme {
        word: "venom",
        name: "Venom",
        stops: [Rgb(255, 60, 172), Rgb(150, 82, 255), Rgb(43, 134, 255)],
    },
    Theme {
        word: "blood",
        name: "Blood",
        stops: [Rgb(255, 128, 64), Rgb(255, 36, 84), Rgb(184, 16, 112)],
    },
    Theme {
        word: "ocean",
        name: "Ocean",
        stops: [Rgb(0, 242, 254), Rgb(64, 156, 255), Rgb(122, 92, 255)],
    },
    Theme {
        word: "toxic",
        name: "Toxic",
        stops: [Rgb(216, 255, 64), Rgb(48, 255, 140), Rgb(0, 196, 255)],
    },
    Theme {
        word: "sunset",
        name: "Sunset",
        stops: [Rgb(255, 214, 64), Rgb(255, 106, 72), Rgb(232, 52, 160)],
    },
    Theme {
        word: "ghost",
        name: "Ghost",
        stops: [Rgb(255, 255, 255), Rgb(176, 184, 204), Rgb(108, 116, 140)],
    },
];

/// The theme `spider.json` names, Venom when it names none.
pub fn theme(word: &str) -> &'static Theme {
    let word = word.trim().to_ascii_lowercase();
    THEMES
        .iter()
        .find(|theme| theme.word == word)
        .unwrap_or(&THEMES[0])
}

impl Theme {
    /// The gradient at `t` (0 = left, 1 = right).
    pub fn at(&self, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0) * 2.0;
        if t <= 1.0 {
            blend(self.stops[0], self.stops[1], t)
        } else {
            blend(self.stops[1], self.stops[2], t - 1.0)
        }
    }

    /// The accent colour: selection markers, focused fields.
    pub fn accent(&self) -> Rgb {
        self.at(0.3)
    }

    /// The position of this theme in [`THEMES`].
    pub fn index(&self) -> usize {
        THEMES
            .iter()
            .position(|theme| theme.word == self.word)
            .unwrap_or(0)
    }
}

/// How one character looks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Style {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub underline: bool,
}

impl Style {
    pub fn fg(colour: Rgb) -> Style {
        Style {
            fg: Some(colour),
            ..Style::default()
        }
    }

    pub fn bold(mut self) -> Style {
        self.bold = true;
        self
    }

    pub fn on(mut self, colour: Rgb) -> Style {
        self.bg = Some(colour);
        self
    }

    fn sgr(self, out: &mut String) {
        out.push_str("\x1b[0");
        if self.bold {
            out.push_str(";1");
        }
        if self.underline {
            out.push_str(";4");
        }
        if let Some(Rgb(r, g, b)) = self.fg {
            let _ = write!(out, ";38;2;{r};{g};{b}");
        }
        if let Some(Rgb(r, g, b)) = self.bg {
            let _ = write!(out, ";48;2;{r};{g};{b}");
        }
        out.push('m');
    }
}

/// One character of the canvas.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            ch: ' ',
            style: Style::default(),
        }
    }
}

/// The number of columns a text takes (the window only uses one-column characters).
pub fn width(text: &str) -> i32 {
    text.chars().count() as i32
}

/// A text cut to `max` columns, with an ellipsis when it was longer.
pub fn clip(text: &str, max: i32) -> String {
    if max <= 0 {
        return String::new();
    }
    if width(text) <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take((max - 1) as usize).collect();
    cut.push('…');
    cut
}

/// A grid of characters the size of the window.
pub struct Canvas {
    pub w: i32,
    pub h: i32,
    cells: Vec<Cell>,
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Canvas {
        let (w, h) = (w.max(1), h.max(1));
        Canvas {
            w,
            h,
            cells: vec![Cell::default(); (w * h) as usize],
        }
    }

    pub fn put(&mut self, x: i32, y: i32, ch: char, style: Style) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let cell = &mut self.cells[(y * self.w + x) as usize];
        // A cell drawn without a background keeps the one already under it (selection bars).
        let bg = style.bg.or(cell.style.bg);
        *cell = Cell {
            ch,
            style: Style { bg, ..style },
        };
    }

    /// Writes `text` from `x`; returns the column after it.
    pub fn text(&mut self, x: i32, y: i32, text: &str, style: Style) -> i32 {
        let mut column = x;
        for ch in text.chars() {
            self.put(column, y, ch, style);
            column += 1;
        }
        column
    }

    /// Writes `text` centred on the row.
    pub fn centre(&mut self, y: i32, text: &str, style: Style) {
        let x = (self.w - width(text)) / 2;
        self.text(x, y, text, style);
    }

    /// Writes `text` in bold with the theme's whole gradient across it.
    pub fn gradient(&mut self, x: i32, y: i32, text: &str, theme: &Theme) -> i32 {
        let count = width(text).max(2) - 1;
        let mut column = x;
        for (index, ch) in text.chars().enumerate() {
            let t = index as f32 / count as f32;
            self.put(column, y, ch, Style::fg(theme.at(t)).bold());
            column += 1;
        }
        column
    }

    /// A horizontal line of `ch`, coloured along the gradient and fading at both ends.
    pub fn rule(&mut self, x: i32, y: i32, length: i32, theme: &Theme, strength: f32) {
        for i in 0..length.max(0) {
            let t = i as f32 / (length - 1).max(1) as f32;
            // Fades in and out at the ends.
            let edge = (t.min(1.0 - t) * 6.0).min(1.0);
            let colour = theme.at(t).shade(1.0 - strength * edge);
            self.put(x + i, y, '─', Style::fg(colour));
        }
    }

    /// A plain horizontal line.
    pub fn hline(&mut self, x: i32, y: i32, length: i32, ch: char, style: Style) {
        for i in 0..length.max(0) {
            self.put(x + i, y, ch, style);
        }
    }

    /// A box outline.
    pub fn frame(&mut self, x: i32, y: i32, w: i32, h: i32, style: Style) {
        if w < 2 || h < 2 {
            return;
        }
        self.put(x, y, '┌', style);
        self.put(x + w - 1, y, '┐', style);
        self.put(x, y + h - 1, '└', style);
        self.put(x + w - 1, y + h - 1, '┘', style);
        self.hline(x + 1, y, w - 2, '─', style);
        self.hline(x + 1, y + h - 1, w - 2, '─', style);
        for row in y + 1..y + h - 1 {
            self.put(x, row, '│', style);
            self.put(x + w - 1, row, '│', style);
        }
    }

    /// Paints the background of an area.
    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, colour: Rgb) {
        for row in y.max(0)..(y + h).min(self.h) {
            for column in x.max(0)..(x + w).min(self.w) {
                let cell = &mut self.cells[(row * self.w + column) as usize];
                cell.style.bg = Some(colour);
            }
        }
    }

    /// The character at a place (tests and snapshots).
    pub fn at(&self, x: i32, y: i32) -> Option<Cell> {
        (x >= 0 && y >= 0 && x < self.w && y < self.h).then(|| self.cells[(y * self.w + x) as usize])
    }

    /// One row as terminal output: characters and colour codes, reset at the end.
    pub fn row(&self, y: i32) -> String {
        let mut out = String::with_capacity(self.w as usize * 2);
        let mut current: Option<Style> = None;
        for cell in &self.cells[(y * self.w) as usize..((y + 1) * self.w) as usize] {
            if current != Some(cell.style) {
                cell.style.sgr(&mut out);
                current = Some(cell.style);
            }
            out.push(cell.ch);
        }
        out.push_str("\x1b[0m");
        out
    }

    /// One row as plain text (tests and snapshots).
    pub fn plain(&self, y: i32) -> String {
        self.cells[(y * self.w) as usize..((y + 1) * self.w) as usize]
            .iter()
            .map(|cell| cell.ch)
            .collect::<String>()
            .trim_end()
            .to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklab_round_trips() {
        for colour in [Rgb(255, 60, 172), Rgb(0, 0, 0), Rgb(255, 255, 255), Rgb(43, 134, 255)] {
            let back = from_oklab(to_oklab(colour));
            for (a, b) in [(colour.0, back.0), (colour.1, back.1), (colour.2, back.2)] {
                assert!(a.abs_diff(b) <= 1, "{colour:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn gradients_start_and_end_on_their_stops() {
        for each in &THEMES {
            assert_eq!(each.at(0.0), each.stops[0]);
            assert_eq!(each.at(1.0), each.stops[2]);
            assert_eq!(theme(each.word).word, each.word);
        }
        assert_eq!(theme("nope").word, "venom");
    }

    #[test]
    fn the_canvas_clips_and_renders_rows() {
        let mut canvas = Canvas::new(10, 2);
        canvas.text(7, 0, "abcdef", Style::fg(ink::TEXT));
        canvas.text(-2, 1, "xyz", Style::default());
        assert_eq!(canvas.plain(0), "       abc");
        assert_eq!(canvas.plain(1), "z");
        assert!(canvas.row(0).ends_with("\x1b[0m"));
        assert_eq!(clip("Bonjour tout le monde", 8), "Bonjour…");
        assert_eq!(clip("court", 8), "court");
    }
}
