//! The console: raw keys, the alternate screen, and drawing a [`Canvas`] row by row.

use std::fmt::Write as _;
use std::io::{self, Write};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, terminal};

use super::paint::Canvas;

/// A key, as the screens read it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Esc,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Char(char),
    /// Ctrl + a letter (never AltGr, which Windows reports as Ctrl + Alt).
    Ctrl(char),
    /// Text pasted at once (Ctrl+V).
    Paste(String),
    /// The window changed size: everything is drawn again.
    Resize,
}

/// The console, from the first frame to the last.
pub struct Term {
    out: io::Stdout,
    /// What each row shows now, to send only the rows that change.
    shown: Vec<String>,
    pub w: u16,
    pub h: u16,
    open: bool,
    /// The window's title right now.
    titled: String,
}

impl Term {
    /// Takes the console over: raw keys, alternate screen, hidden cursor, window title.
    pub fn open(title: &str) -> io::Result<Term> {
        #[cfg(windows)]
        if !crossterm::ansi_support::supports_ansi() {
            return Err(io::Error::other("this console has no colour support"));
        }
        let (w, h) = terminal::size()?;
        terminal::enable_raw_mode()?;
        let mut out = io::stdout();
        if let Err(error) = execute!(
            out,
            terminal::EnterAlternateScreen,
            cursor::Hide,
            terminal::SetTitle(title),
            terminal::Clear(terminal::ClearType::All)
        ) {
            let _ = terminal::disable_raw_mode();
            return Err(error);
        }
        Ok(Term {
            out,
            shown: Vec::new(),
            w,
            h,
            open: true,
            titled: title.to_owned(),
        })
    }

    /// A blank canvas the size of the window. The last column is left alone: writing there
    /// makes some consoles wrap and scroll.
    pub fn canvas(&self) -> Canvas {
        Canvas::new(i32::from(self.w) - 1, i32::from(self.h))
    }

    /// Shows a canvas, sending only the rows that changed; `cursor` places the blinking
    /// cursor (a text field), `None` hides it.
    pub fn present(&mut self, canvas: &Canvas, cursor: Option<(i32, i32)>) -> io::Result<()> {
        let mut out = String::new();
        out.push_str("\x1b[?25l");
        if self.shown.len() != canvas.h as usize {
            self.shown = vec![String::new(); canvas.h as usize];
        }
        for y in 0..canvas.h {
            let row = canvas.row(y);
            if self.shown[y as usize] != row {
                let _ = write!(out, "\x1b[{};1H", y + 1);
                out.push_str(&row);
                self.shown[y as usize] = row;
            }
        }
        if let Some((x, y)) = cursor {
            let _ = write!(out, "\x1b[{};{}H\x1b[?25h", y + 1, x + 1);
        }
        self.out.write_all(out.as_bytes())?;
        self.out.flush()
    }

    /// Sets the window's title (shown in the taskbar), only when it changes.
    pub fn title(&mut self, title: &str) {
        if self.titled != title {
            self.titled = title.to_owned();
            let _ = execute!(self.out, terminal::SetTitle(title));
        }
    }

    /// Forgets what is on screen, so the next frame is drawn whole.
    pub fn redraw(&mut self) {
        self.shown.clear();
        let _ = self.out.write_all(b"\x1b[0m\x1b[2J");
    }

    /// The next key, or `None` once `timeout` is over (or for an event no screen reads).
    pub fn key(&mut self, timeout: Duration) -> io::Result<Option<Key>> {
        if !event::poll(timeout)? {
            return Ok(None);
        }
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => Ok(map(key)),
            Event::Resize(w, h) => {
                self.w = w;
                self.h = h;
                self.redraw();
                Ok(Some(Key::Resize))
            }
            _ => Ok(None),
        }
    }

    /// Gives the console back: normal screen, cursor, cooked keys.
    pub fn close(&mut self) {
        if !self.open {
            return;
        }
        self.open = false;
        let _ = execute!(self.out, cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        self.close();
    }
}

/// Puts the console back after a crash, before the panic message is printed.
pub fn restore() {
    let mut out = io::stdout();
    let _ = execute!(out, cursor::Show, terminal::LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();
}

fn map(key: KeyEvent) -> Option<Key> {
    // AltGr arrives as Ctrl + Alt: it types @ # { [ | on AZERTY keyboards, so it is a
    // character, not a shortcut.
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT);
    Some(match key.code {
        KeyCode::Char(c) if ctrl => match c.to_ascii_lowercase() {
            'v' => Key::Paste(clipboard()?),
            other => Key::Ctrl(other),
        },
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Tab => Key::Tab,
        KeyCode::BackTab => Key::BackTab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        _ => return None,
    })
}

/// The clipboard's text (Windows Terminal pastes by itself; the old console host leaves
/// Ctrl+V to the program).
#[cfg(windows)]
fn clipboard() -> Option<String> {
    const CF_UNICODETEXT: u32 = 13;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(owner: *mut core::ffi::c_void) -> i32;
        fn CloseClipboard() -> i32;
        fn GetClipboardData(format: u32) -> *mut core::ffi::c_void;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalLock(memory: *mut core::ffi::c_void) -> *mut u16;
        fn GlobalUnlock(memory: *mut core::ffi::c_void) -> i32;
    }
    // SAFETY: the clipboard is opened and closed here; the locked block is a NUL-terminated
    // UTF-16 string while it stays locked, and it is read before being unlocked.
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return None;
        }
        let handle = GetClipboardData(CF_UNICODETEXT);
        let mut text = None;
        if !handle.is_null() {
            let data = GlobalLock(handle);
            if !data.is_null() {
                let mut length = 0;
                while *data.add(length) != 0 && length < 1 << 16 {
                    length += 1;
                }
                text = Some(String::from_utf16_lossy(std::slice::from_raw_parts(data, length)));
                GlobalUnlock(handle);
            }
        }
        CloseClipboard();
        text
    }
}

#[cfg(not(windows))]
fn clipboard() -> Option<String> {
    None
}
