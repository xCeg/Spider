//! A one-line text box: typing, the cursor keys, Ctrl+U, pasting.

use super::term::Key;

/// A one-line text box.
#[derive(Default)]
pub(super) struct Input {
    pub(super) chars: Vec<char>,
    pub(super) cursor: usize,
}

impl Input {
    pub(super) fn set(&mut self, text: &str) {
        self.chars = text.chars().collect();
        self.cursor = self.chars.len();
    }

    pub(super) fn value(&self) -> String {
        self.chars.iter().collect()
    }

    pub(super) fn insert(&mut self, ch: char) {
        if !ch.is_control() {
            self.chars.insert(self.cursor, ch);
            self.cursor += 1;
        }
    }

    pub(super) fn paste(&mut self, text: &str) {
        for ch in text.chars() {
            match ch {
                '\r' | '\n' => {}
                '\t' => self.insert(' '),
                other => self.insert(other),
            }
        }
    }

    /// Edits the box; `false` when the key is not an editing key.
    pub(super) fn edit(&mut self, key: &Key) -> bool {
        match key {
            Key::Char(ch) => self.insert(*ch),
            Key::Paste(text) => self.paste(text),
            Key::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                self.chars.remove(self.cursor);
            }
            Key::Delete if self.cursor < self.chars.len() => {
                self.chars.remove(self.cursor);
            }
            Key::Left => self.cursor = self.cursor.saturating_sub(1),
            Key::Right => self.cursor = (self.cursor + 1).min(self.chars.len()),
            Key::Home => self.cursor = 0,
            Key::End => self.cursor = self.chars.len(),
            Key::Ctrl('u') => {
                self.chars.clear();
                self.cursor = 0;
            }
            Key::Backspace | Key::Delete => {}
            _ => return false,
        }
        true
    }
}
