# Spider: notes for Claude

Spider = Discord Rich Presence without a token: a Rust console app (`spider.exe`, copied to
`Spider.exe` by `build.bat`) with an animated window (crossterm) and the same engine as
Cinnabug's DiscordRPC (`cinnabar-perso`: `crates/rich-presence/src/custom.rs`, extracted as
`standalone/rpc-presence` in v238.1). The user is French and not a developer: the window, the
log, the README and the error messages are in French; do the work, push it, explain simply.
No AI model identifier in commits or code.

## Layout
- `src/engine.rs`: the transports. Pipe (`\\.\pipe\discord-ipc-N`, kept open per application in
  `PIPES`) and debug port (CDP on 9222, `LOCAL_ACTIVITY_UPDATE` through the Flux dispatcher,
  socket ids `spider1..3`, old `rpc-presence1..3` cleared too). `publish(cards, mode, previous,
  handover)`: one transport for every card, sticky; only a real change (`handover`) clears first.
- `src/config.rs`: `spider.json` (`Settings`, `Activity`, `save` through a temp file, `warnings`).
- `src/runner.rs`: the loop (`Runner::step`), `Log` (local time, no repeated line), `Worker`
  (the loop on a thread for the window), `clear_on_close` (console close handler), retry
  backoff (retry x2 per failure, 60 s max).
- `src/doctor.rs`: the diagnostic (CLI `spider doctor` and the window).
- `src/ui/`: `term.rs` (raw keys, alternate screen, row-diff drawing, AltGr = characters,
  Ctrl+V reads the clipboard), `paint.rs` (canvas, OKLab gradients, 6 themes), `logo.rs` (ANSI
  Shadow logo, opening animation, header/footer), `app.rs` (menu, dashboard, diagnostic,
  themes, dialogs, probe thread), `wizard.rs` (hub + one-question-at-a-time card wizard,
  templates), `preview.rs` (a Discord card drawn in characters).
- Glyphs: only WGL4 ones (Consolas in the old console host has no ✓, ━ or rounded corners).

## Discord facts measured on the user's PC (2026-11-16)
- Discord answers about **two quick handshakes per application**, then stays silent (no READY,
  no close) for about a minute. Never hop to another pipe on silence (it may be another
  account); keep connections open; the window's probe never shakes hands (it reads each
  pipe's server process with `GetNamedPipeServerProcessId`, which does not count); the
  diagnostic shakes hands as an application the cards do not use, once per Discord process.
  The old engine closed the pipe on every refresh, which hit this limit: that was the
  "statuses vanish and come back forever" bug.
- Windows takes **~2 s to refuse** a connection to a closed local port: a refusal of the debug
  port is cached for 4 s (`REFUSED_AT`).
- Application `880218394199220334` (Cinnabug's "YouTube" preset) is Discord's **Watch Together**;
  the template forces the title "YouTube" (debug port only).
- The renderer shows a CDP activity's `name` as is: an empty one is filled from the public API
  (`application_name`, `curl.exe`, `/applications/{id}/rpc`, cached).
- Texts: one character is padded with U+3164, more than 128 cut; labels 32, links 512.

## Checking a change
- `cargo test`, `cargo clippy --all-targets`, then `build.bat` (or `cargo build --release`).
- Look at the real window without opening one: `tools/snapper` runs `Spider.exe` in a
  pseudo-console, sends keys and saves each screen as HTML + text:
  `snapper <exe> <out> 120x30 -- --config <tmp>\spider.json ++ w:2300 s:menu k:{enter} ...`
  (steps: `w:ms`, `k:keys` with `{up} {down} {enter} {esc} {tab} {bs} {ctrl-u} {ctrl-s}`,
  `s:name`, `a:name:count:ms`). Use a config whose client is closed (e.g. `"client": "canary"`)
  to test the dashboard without publishing on the user's accounts.
- In this environment cmd does not run a `.bat` from the current folder
  (`NoDefaultCurrentDirectoryInExePath`): call `build.bat` by its full path.
- `build.bat` must stay CRLF (`.gitattributes`).
