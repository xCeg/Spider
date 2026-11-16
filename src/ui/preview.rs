//! A Discord activity card drawn in characters: roughly what the profile will show.

use super::paint::{Canvas, Style, Theme, clip, ink, width};

/// What a card shows.
pub(super) struct CardView {
    pub kind: u8,
    pub title: String,
    pub details: String,
    pub state: String,
    pub image: bool,
    pub small_image: bool,
    pub platform: Option<&'static str>,
    pub buttons: Vec<String>,
    /// The seconds on the clock, `None` without a timer.
    pub elapsed: Option<u64>,
}

pub(super) const WIDTH: i32 = 38;

pub(super) fn height(card: &CardView) -> i32 {
    8 + card.buttons.len() as i32 * 2
}

/// The word Discord writes above the card.
pub(super) fn heading(kind: u8) -> &'static str {
    match kind {
        1 => "EN STREAM",
        2 => "ÉCOUTE",
        3 => "REGARDE",
        5 => "EN COMPÉTITION",
        _ => "JOUE À",
    }
}

/// The verb the window uses for an activity type.
pub(super) fn verb(kind: u8) -> &'static str {
    match kind {
        1 => "Streame",
        2 => "Écoute",
        3 => "Regarde",
        5 => "Participe à",
        _ => "Joue à",
    }
}

/// `12:34`, or `1:02:03` past an hour.
pub(super) fn clock(seconds: u64) -> String {
    if seconds >= 3600 {
        format!("{}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
    } else {
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }
}

/// Draws the card with its top-left corner at (`x`, `y`).
pub(super) fn draw(canvas: &mut Canvas, x: i32, y: i32, card: &CardView, theme: &Theme) {
    canvas.fill(x, y, WIDTH, height(card), ink::CARD);
    canvas.text(x + 2, y + 1, heading(card.kind), Style::fg(ink::MUTED).bold());
    if let Some(platform) = card.platform {
        let badge = match platform {
            "xbox" => "XBOX",
            "ps5" => "PS5",
            _ => "VR",
        };
        canvas.text(
            x + WIDTH - 2 - width(badge),
            y + 1,
            badge,
            Style::fg(theme.accent()).bold(),
        );
    }
    // The picture: the theme's gradient when there is one, an empty frame otherwise.
    for row in 0..4 {
        for column in 0..8 {
            let (ch, colour) = if card.image {
                ('█', theme.at(column as f32 / 7.0).shade(row as f32 * 0.09))
            } else {
                ('░', ink::FAINT)
            };
            canvas.put(x + 2 + column, y + 3 + row, ch, Style::fg(colour));
        }
    }
    if card.small_image {
        canvas.put(x + 9, y + 6, '●', Style::fg(ink::TEXT).on(ink::CARD));
    }
    let left = x + 12;
    let room = WIDTH - 14;
    let mut line = y + 3;
    canvas.text(left, line, &clip(&card.title, room), Style::fg(ink::TEXT).bold());
    line += 1;
    for (text, colour) in [(&card.details, ink::TEXT), (&card.state, ink::MUTED)] {
        if !text.trim().is_empty() {
            canvas.text(left, line, &clip(text.trim(), room), Style::fg(colour));
            line += 1;
        }
    }
    if let Some(seconds) = card.elapsed {
        if card.kind == 2 {
            // A listening card shows a playback bar (four minutes, then it starts over).
            let at = seconds % 240;
            let bar = room - 10;
            let filled = (at as i32 * bar) / 240;
            let after = canvas.text(left, line, &format!("{}:{:02}", at / 60, at % 60), Style::fg(ink::MUTED));
            for i in 0..bar {
                let colour = if i < filled { ink::TEXT } else { ink::GHOST };
                canvas.put(after + 1 + i, line, '─', Style::fg(colour));
            }
            canvas.text(after + 2 + bar, line, "4:00", Style::fg(ink::MUTED));
        } else {
            canvas.text(
                left,
                line,
                &format!("{} écoulées", clock(seconds)),
                Style::fg(ink::GOOD),
            );
        }
    }
    let mut row = y + 8;
    for label in &card.buttons {
        canvas.fill(x + 2, row, WIDTH - 4, 1, ink::BUTTON);
        let label = clip(label, WIDTH - 6);
        canvas.text(
            x + (WIDTH - width(&label)) / 2,
            row,
            &label,
            Style::fg(ink::TEXT).bold(),
        );
        row += 2;
    }
}
