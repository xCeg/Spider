//! Runs a console program in a pseudo-console, sends it keys and saves the screen as HTML.
//!
//! usage: snapper <exe> <out_dir> <cols>x<rows> -- <program args...> ++ <steps...>
//! steps: w:MS (wait), k:KEYS (send keys, {up} {down} {left} {right} {enter} {esc} {tab}
//!        {bs} {del} {ctrl-u} {ctrl-s}), s:NAME (snapshot), a:NAME:COUNT:MS (frames)

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

fn keys(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix('{')
            && let Some(end) = stripped.find('}')
        {
            let name = &stripped[..end];
            out.extend_from_slice(match name {
                "up" => b"\x1b[A".as_slice(),
                "down" => b"\x1b[B",
                "right" => b"\x1b[C",
                "left" => b"\x1b[D",
                "enter" => b"\r",
                "esc" => b"\x1b",
                "tab" => b"\t",
                "bs" => b"\x7f",
                "del" => b"\x1b[3~",
                "ctrl-u" => b"\x15",
                "ctrl-s" => b"\x13",
                "home" => b"\x1b[H",
                "end" => b"\x1b[F",
                other => panic!("unknown key {other}"),
            });
            rest = &stripped[end + 1..];
        } else {
            let ch = rest.chars().next().unwrap();
            let mut buffer = [0u8; 4];
            out.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
            rest = &rest[ch.len_utf8()..];
        }
    }
    out
}

fn colour(colour: vt100::Color, fallback: &str) -> String {
    const BASE: [&str; 16] = [
        "#0c0c0c", "#c50f1f", "#13a10e", "#c19c00", "#0037da", "#881798", "#3a96dd", "#cccccc",
        "#767676", "#e74856", "#16c60c", "#f9f1a5", "#3b78ff", "#b4009e", "#61d6d6", "#f2f2f2",
    ];
    match colour {
        vt100::Color::Default => fallback.to_owned(),
        vt100::Color::Idx(index) if index < 16 => BASE[index as usize].to_owned(),
        vt100::Color::Idx(index) => format!("var(--i{index}, #888)"),
        vt100::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
    }
}

fn html(screen: &vt100::Screen, title: &str) -> (String, String) {
    let (rows, cols) = screen.size();
    let (cursor_row, cursor_col) = screen.cursor_position();
    let show_cursor = !screen.hide_cursor();
    let mut out = format!(
        "<!doctype html><meta charset=utf-8><title>{title}</title><style>\
         body{{background:#0c0c0c;margin:0;padding:18px}}\
         pre{{font-family:'Cascadia Mono','Cascadia Code',Consolas,monospace;font-size:12px;\
         line-height:15px;color:#cccccc;margin:0}} span{{white-space:pre}}</style><pre>"
    );
    let mut plain = String::new();
    for row in 0..rows {
        let mut current = String::new();
        let mut run = String::new();
        let mut line = String::new();
        for col in 0..cols {
            let Some(cell) = screen.cell(row, col) else { continue };
            if cell.is_wide_continuation() {
                continue;
            }
            let mut fg = colour(cell.fgcolor(), "#cccccc");
            let mut bg = colour(cell.bgcolor(), "transparent");
            if cell.inverse() || (show_cursor && row == cursor_row && col == cursor_col) {
                std::mem::swap(&mut fg, &mut bg);
                if fg == "transparent" {
                    fg = "#0c0c0c".to_owned();
                }
                if bg == "transparent" {
                    bg = "#cccccc".to_owned();
                }
            }
            let style = format!(
                "color:{fg};background:{bg};font-weight:{}",
                if cell.bold() { "bold" } else { "normal" }
            );
            let text = if cell.has_contents() { cell.contents().to_owned() } else { " ".to_owned() };
            line.push_str(&text);
            if style != current {
                if !run.is_empty() {
                    out.push_str(&format!("<span style=\"{current}\">{run}</span>"));
                    run.clear();
                }
                current = style;
            }
            run.push_str(&text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"));
        }
        if !run.is_empty() {
            out.push_str(&format!("<span style=\"{current}\">{run}</span>"));
        }
        out.push('\n');
        plain.push_str(line.trim_end());
        plain.push('\n');
    }
    out.push_str("</pre>");
    (out, plain)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let exe = &args[0];
    let out_dir = std::path::PathBuf::from(&args[1]);
    let (cols, rows) = args[2].split_once('x').unwrap();
    let (cols, rows): (u16, u16) = (cols.parse().unwrap(), rows.parse().unwrap());
    let split = args.iter().position(|arg| arg == "++").unwrap();
    let program_args = &args[4..split];
    let steps = &args[split + 1..];
    std::fs::create_dir_all(&out_dir).unwrap();

    let pty = native_pty_system()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .unwrap();
    let mut command = CommandBuilder::new(exe);
    for arg in program_args {
        command.arg(arg);
    }
    if let Some(folder) = std::path::Path::new(exe).parent() {
        command.cwd(folder);
    }
    let mut child = pty.slave.spawn_command(command).unwrap();
    drop(pty.slave);
    let mut reader = pty.master.try_clone_reader().unwrap();
    let writer = Arc::new(Mutex::new(pty.master.take_writer().unwrap()));
    let answer = Arc::clone(&writer);
    let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 0)));
    let feed = Arc::clone(&parser);
    let raw = Arc::new(Mutex::new(Vec::<u8>::new()));
    let raw_feed = Arc::clone(&raw);
    std::thread::spawn(move || {
        let mut buffer = [0u8; 65536];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    // ConPTY asks where the cursor is before it shows anything.
                    if buffer[..read].windows(4).any(|w| w == b"\x1b[6n") {
                        let mut answer = answer.lock().unwrap();
                        let _ = answer.write_all(b"\x1b[1;1R");
                        let _ = answer.flush();
                    }
                    feed.lock().unwrap().process(&buffer[..read]);
                    raw_feed.lock().unwrap().extend_from_slice(&buffer[..read]);
                }
            }
        }
    });
    for step in steps {
        let (kind, value) = step.split_once(':').unwrap();
        match kind {
            "w" => std::thread::sleep(Duration::from_millis(value.parse().unwrap())),
            "k" => {
                let mut writer = writer.lock().unwrap();
                writer.write_all(&keys(value)).unwrap();
                writer.flush().unwrap();
                drop(writer);
                std::thread::sleep(Duration::from_millis(60));
            }
            "s" => {
                let parser = parser.lock().unwrap();
                let (page, plain) = html(parser.screen(), value);
                std::fs::write(out_dir.join(format!("{value}.html")), page).unwrap();
                std::fs::write(out_dir.join(format!("{value}.txt")), plain).unwrap();
            }
            "a" => {
                let mut parts = value.split(':');
                let name = parts.next().unwrap();
                let count: usize = parts.next().unwrap().parse().unwrap();
                let every: u64 = parts.next().unwrap().parse().unwrap();
                for index in 0..count {
                    {
                        let parser = parser.lock().unwrap();
                        let (page, plain) = html(parser.screen(), name);
                        std::fs::write(out_dir.join(format!("{name}_{index:02}.html")), page).unwrap();
                        std::fs::write(out_dir.join(format!("{name}_{index:02}.txt")), plain).unwrap();
                    }
                    std::thread::sleep(Duration::from_millis(every));
                }
            }
            other => panic!("unknown step {other}"),
        }
    }
    let _ = child.kill();
    std::fs::write(out_dir.join("raw.bin"), raw.lock().unwrap().as_slice()).unwrap();
}
