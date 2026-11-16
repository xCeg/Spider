//! Spider's Discord Rich Presence engine: no bot token, no user token.
//!
//! Discord's local RPC pipe only accepts activity types 0/2/3/5 and forces the
//! platform to `desktop`. The client's own debug port (CDP, started with
//! `--remote-debugging-port=9222`) exposes the renderer, where the internal
//! `LOCAL_ACTIVITY_UPDATE` event carries STREAMING, console glyphs and buttons.
//!
//! The pipe is still used to sign image URLs (`mp:external/...`) and for the
//! plain RPC fallback when the debug port is closed.
//!
//! Every card goes on one transport at a time. Auto remembers the transport that
//! worked, so a slow renderer tick can no longer make the profile alternate between
//! the CDP card (Streaming, badges, buttons) and the pipe card (Playing).
//!
//! Several Discord clients can run at once (Discord, PTB, Canary), each logged into its own
//! account and owning its own pipe (`discord-ipc-0`, `-1`...). [`set_client`] picks the one the
//! cards go to; Auto keeps the first pipe that answers, as before.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// Discord's debug port (start Discord with `--remote-debugging-port=9222`).
pub const CDP_PORT: u16 = 9222;

/// A fake Listening track lasts this long, then the progress bar starts over.
const LISTEN_MILLIS: u64 = 240_000;

/// Spotify's own application: the one that already wears the Spotify icon.
pub const SPOTIFY_APP_ID: u64 = 668_500_067_572_776_972;

/// How long a signed `mp:external/...` path is reused. Discord re-signs the asset from
/// time to time; keeping a path forever made the picture disappear after a client restart.
const SIGNED_FOR: Duration = Duration::from_secs(600);

/// Bumped whenever the Discord pipe is (re)connected: a new client may sign assets
/// differently, so the cached `mp:external/...` paths must not survive a restart.
static SIGN_GENERATION: AtomicU64 = AtomicU64::new(0);

const TIMEOUT: Duration = Duration::from_secs(3);

/// Discord's limits for a card's texts. The pipe refuses the whole SET_ACTIVITY (code 4000)
/// when one text is a single character or longer than 128.
const TEXT_MAX: usize = 128;
/// A button label holds 32 characters at most, its link 512.
const LABEL_MAX: usize = 32;
const URL_MAX: usize = 512;
/// An invisible Hangul filler: pads a one-character text up to Discord's two-character minimum.
const FILLER: char = '\u{3164}';

/// The CDP socket ids of the three slots, and the ones the tool used before it became Spider
/// (cleared too, so a card left by the old name does not stay on the profile).
const SOCKETS: [&str; 3] = ["spider1", "spider2", "spider3"];
const OLD_SOCKETS: [&str; 3] = ["rpc-presence1", "rpc-presence2", "rpc-presence3"];

/// Where the card goes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    /// The debug port when Discord is running with it, the pipe otherwise.
    Auto,
    /// Only the debug port (Streaming, glyphs and buttons).
    Cdp,
    /// Only the local pipe (no Streaming nor glyph).
    Rpc,
}

/// Which transport accepted the last push.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Transport {
    Cdp,
    Rpc,
}

impl Transport {
    fn other(self) -> Transport {
        match self {
            Transport::Cdp => Transport::Rpc,
            Transport::Rpc => Transport::Cdp,
        }
    }

    /// How the window and the log name it.
    pub fn label(self) -> &'static str {
        match self {
            Transport::Cdp => "port de debug",
            Transport::Rpc => "pipe",
        }
    }
}

/// The Discord client (and so the account) the cards go to.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Client {
    /// The first pipe that answers, whatever its client.
    #[default]
    Auto,
    Stable,
    Ptb,
    Canary,
}

impl Client {
    pub const ALL: [Client; 4] = [Client::Auto, Client::Stable, Client::Ptb, Client::Canary];

    /// `spider.json`'s word: `auto`, `stable`, `ptb` or `canary`.
    pub fn parse(word: &str) -> Client {
        match word.trim().to_ascii_lowercase().as_str() {
            "stable" | "discord" => Client::Stable,
            "ptb" => Client::Ptb,
            "canary" => Client::Canary,
            _ => Client::Auto,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Client::Auto => "auto",
            Client::Stable => "stable",
            Client::Ptb => "ptb",
            Client::Canary => "canary",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Client::Auto => "Auto",
            Client::Stable => "Discord",
            Client::Ptb => "Discord PTB",
            Client::Canary => "Discord Canary",
        }
    }

    fn code(self) -> u8 {
        match self {
            Client::Auto => 0,
            Client::Stable => 1,
            Client::Ptb => 2,
            Client::Canary => 3,
        }
    }

    fn from_code(code: u8) -> Client {
        match code {
            1 => Client::Stable,
            2 => Client::Ptb,
            3 => Client::Canary,
            _ => Client::Auto,
        }
    }

    /// The client behind a renderer page's host (`ptb.discord.com`...).
    fn of_host(host: &str) -> Client {
        if host.starts_with("ptb.") {
            Client::Ptb
        } else if host.starts_with("canary.") {
            Client::Canary
        } else {
            Client::Stable
        }
    }

    /// The client behind a pipe, from its READY event (`//ptb.discord.com/api`...).
    fn of_ready(ready: &Value) -> Client {
        let endpoint = ready
            .pointer("/data/config/api_endpoint")
            .and_then(Value::as_str)
            .unwrap_or_default();
        Client::of_host(endpoint.trim_start_matches('/'))
    }

    /// Its folder in `%LOCALAPPDATA%` and its executable.
    pub fn install(self) -> Option<(&'static str, &'static str)> {
        match self {
            Client::Auto => None,
            Client::Stable => Some(("Discord", "Discord.exe")),
            Client::Ptb => Some(("DiscordPTB", "DiscordPTB.exe")),
            Client::Canary => Some(("DiscordCanary", "DiscordCanary.exe")),
        }
    }
}

static CLIENT: AtomicU8 = AtomicU8::new(0);

/// The client the cards go to.
pub fn client() -> Client {
    Client::from_code(CLIENT.load(Ordering::Relaxed))
}

/// Chooses the client the cards go to; `true` when it changed.
///
/// The open pipes belong to the old client: they are dropped (Discord removes the cards they
/// held) and the pictures they signed with them. The caller clears the debug-port cards first.
pub fn set_client(next: Client) -> bool {
    if CLIENT.swap(next.code(), Ordering::Relaxed) == next.code() {
        return false;
    }
    #[cfg(windows)]
    if let Ok(mut open) = pipes().lock() {
        open.clear();
    }
    SIGN_GENERATION.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut account) = ACCOUNT.lock() {
        *account = None;
    }
    true
}

/// The Discord account a pipe is logged into.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Account {
    pub client: Client,
    /// The display name (or the username when there is none).
    pub name: String,
    /// `discord-ipc-N`.
    pub pipe: u8,
}

/// The account behind the pipe the cards last used.
static ACCOUNT: Mutex<Option<Account>> = Mutex::new(None);

/// The account the cards go to, once a pipe was opened.
pub fn account() -> Option<Account> {
    ACCOUNT.lock().ok().and_then(|account| account.clone())
}

fn user_name(ready: &Value) -> String {
    let user = ready.pointer("/data/user");
    let get = |key: &str| {
        user.and_then(|user| user.get(key))
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
    };
    get("global_name")
        .or_else(|| get("username"))
        .unwrap_or("?")
        .to_owned()
}

/// One card, as the DiscordRPC settings describe it.
#[derive(Clone, Debug, Default)]
pub struct Config {
    pub app_id: u64,
    /// Activity slot 1..=3. CDP keeps one local activity per socket, so each slot needs its
    /// own socket id or every dispatch replaces the previous card.
    pub slot: u8,
    /// The application whose pipe signs external image URLs. `0` uses `app_id`.
    ///
    /// One application signs the picture once, so every card can wear the same artwork even
    /// when its own application has no Rich Presence assets.
    pub image_signer: u64,
    pub name: String,
    pub details: String,
    pub state: String,
    pub image: String,
    pub image_text: String,
    /// The corner badge (Twitch/YouTube); signed like the large picture.
    pub small_image: String,
    pub small_text: String,
    /// 0 playing, 1 streaming, 2 listening, 3 watching, 5 competing.
    pub kind: u8,
    /// `xbox`, `ps5` or `meta_quest`; only the debug port carries it.
    pub platform: Option<&'static str>,
    pub stream_url: String,
    pub buttons: Vec<(String, String)>,
    pub timer: bool,
}

impl Config {
    /// Identifies the card, to skip pushes that change nothing.
    ///
    /// The timer is part of it: turning the elapsed time off must republish the card without
    /// its `timestamps` (otherwise Discord keeps the old start time forever).
    pub fn signature(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            self.slot,
            self.app_id,
            self.image_signer,
            self.name,
            self.details,
            self.state,
            self.image,
            self.image_text,
            self.small_image,
            self.small_text,
            self.kind,
            self.platform.unwrap_or_default(),
            self.stream_url,
            self.timer,
            self.buttons
                .iter()
                .map(|(label, url)| format!("{label}>{url}"))
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

/// Pushes the card; `Err` only when neither transport is reachable.
pub fn push(config: &Config, mode: Mode) -> Result<Transport, String> {
    push_all(std::slice::from_ref(config), mode)
}

/// Pushes up to Discord's three activity slots. Each card keeps its own pipe for RPC (Discord
/// ties an activity to the connection that set it) and the renderer keeps every type for CDP.
pub fn push_all(configs: &[Config], mode: Mode) -> Result<Transport, String> {
    publish(configs, mode, None, false)
}

/// Pushes every card on ONE transport and keeps `previous` while it still answers.
///
/// Falling back card by card used to flip the transport on and off every few seconds: the
/// renderer card (Streaming, buttons, console badge) and the pipe card (Playing, no badge)
/// are two different activities, so the profile showed one, then the other, and an old card
/// stayed until its own transport cleared it. A push now either accepts all the cards or
/// none, and a switch clears the transport it leaves first.
///
/// `handover` is set when the cards changed while others were on the profile: everything is
/// cleared first. A plain refresh of the same cards must NOT clear: that took the card off the
/// profile every 15 seconds, closed the pipe (Discord drops a pipe's card with it) and threw
/// the signed pictures away, so each refresh blinked the card and flashed a "..." one while the
/// pictures were signed again.
pub fn publish(
    configs: &[Config],
    mode: Mode,
    previous: Option<Transport>,
    handover: bool,
) -> Result<Transport, String> {
    if configs.is_empty() {
        return Err("aucune carte à publier".to_owned());
    }
    // A hand-over (application, type, platform or transport change) drops every card first.
    // The renderer keys an activity by type, so a switch from Playing to Streaming would
    // otherwise leave the old type on the profile next to the new one.
    if handover {
        let _ = clear_transport(Transport::Cdp);
        let _ = clear_transport(Transport::Rpc);
    }
    let order: Vec<Transport> = match mode {
        Mode::Cdp => vec![Transport::Cdp],
        Mode::Rpc => vec![Transport::Rpc],
        Mode::Auto => match previous {
            Some(Transport::Cdp) => vec![Transport::Cdp, Transport::Rpc],
            Some(Transport::Rpc) => vec![Transport::Rpc, Transport::Cdp],
            // First card of the session: keep the richer transport when it is already open.
            None if available() => vec![Transport::Cdp, Transport::Rpc],
            None => vec![Transport::Rpc, Transport::Cdp],
        },
    };
    let mut errors: Vec<String> = Vec::new();
    for (index, transport) in order.into_iter().enumerate() {
        // Leaving a transport must take its cards with it, or the same slot shows twice.
        if index > 0 {
            let _ = clear_transport(transport.other());
        }
        match push_every(configs, transport) {
            Ok(()) => return Ok(transport),
            Err(error) => {
                // Half a set of cards is worse than none: remove what landed, then try the
                // other transport from a clean profile.
                let _ = clear_transport(transport);
                errors.push(match transport {
                    Transport::Cdp => error,
                    Transport::Rpc => format!("pipe : {error}"),
                });
            }
        }
    }
    // The pipe's reason first: "debug port closed" is the usual state, not the news.
    errors.sort_by_key(|error| !error.starts_with("pipe"));
    if errors.is_empty() {
        errors.push("aucun transport n'a répondu".to_owned());
    }
    Err(errors.join(" · "))
}

/// Every card on one transport, all or nothing.
fn push_every(configs: &[Config], transport: Transport) -> Result<(), String> {
    for config in configs {
        match transport {
            Transport::Cdp => cdp_push(config)?,
            Transport::Rpc => rpc_push(config)?,
        }
    }
    Ok(())
}

fn clear_transport(transport: Transport) -> Result<(), String> {
    match transport {
        Transport::Cdp => cdp_clear(),
        Transport::Rpc => rpc_clear(),
    }
}

/// Removes the card from the profile.
pub fn clear(mode: Mode) -> Result<(), String> {
    let mut last = String::new();
    if mode != Mode::Rpc {
        match cdp_clear() {
            Ok(()) => return Ok(()),
            Err(error) => last = error,
        }
    }
    if mode != Mode::Cdp {
        return match rpc_clear() {
            Ok(()) => Ok(()),
            Err(error) if last.is_empty() => Err(error),
            Err(error) => Err(format!("{last}; {error}")),
        };
    }
    Err(last)
}

/// Removes the card from **both** transports.
///
/// The card is tied to the transport that published it: an RPC card survives a CDP clear
/// (and the other way round), and Discord keeps an activity alive as long as its IPC pipe
/// stays connected. Turning the tool off, or changing transport/application, must therefore
/// clear both or the old card stays on the profile and the new one never appears.
pub fn clear_all() -> Result<(), String> {
    let cdp = cdp_clear();
    let rpc = rpc_clear();
    match (cdp, rpc) {
        (Ok(()), _) | (_, Ok(())) => Ok(()),
        (Err(a), Err(b)) => Err(format!("{a}; {b}")),
    }
}

/// Whether a page of the chosen Discord client answers on the debug port.
pub fn available() -> bool {
    cdp_page().is_ok()
}

/// The client that has the debug port open, whatever the chosen one.
pub fn debug_port_client() -> Option<Client> {
    let body = http_get(CDP_PORT, "/json").ok()?;
    let pages: Value = serde_json::from_str(&body).ok()?;
    pages
        .as_array()?
        .iter()
        .filter_map(|page| page.get("url").and_then(Value::as_str))
        .find_map(main_window_host)
        .map(Client::of_host)
}

/// Restarts Discord with `--remote-debugging-port=9222`; returns the client it relaunched.
///
/// The renderer only exposes the internal activity event when Discord was started with that
/// flag; the pipe refuses Streaming (type 1) and forces the platform to desktop. This closes
/// and relaunches Discord, so it only runs when the user asks for it.
#[cfg(windows)]
pub fn restart_with_debug_port() -> Result<Client, String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    let wanted = client();
    forget_refusal();
    if available() {
        return Ok(debug_port_client().unwrap_or(wanted));
    }
    let local = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA est introuvable")?;
    let local = std::path::PathBuf::from(local);
    // The chosen client, or in Auto the one already running (else the first one installed).
    let order = match wanted {
        Client::Auto => vec![Client::Stable, Client::Ptb, Client::Canary],
        other => vec![other],
    };
    let installed: Vec<(Client, std::path::PathBuf, &str)> = order
        .into_iter()
        .filter_map(|client| {
            let (folder, exe) = client.install()?;
            let path = local.join(folder).join(exe);
            path.is_file().then_some((client, path, exe))
        })
        .collect();
    let (picked, path, exe) = installed
        .iter()
        .find(|(_, _, exe)| discord_running(exe))
        .or_else(|| installed.first())
        .cloned()
        .ok_or_else(|| format!("{} n'est pas installé à l'endroit habituel", wanted.label()))?;
    // A second launch only focuses the running client, without the flag: close it first.
    let _ = std::process::Command::new("taskkill")
        .args(["/IM", exe, "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    for _ in 0..24 {
        if !discord_running(exe) {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    std::process::Command::new(&path)
        .arg(format!("--remote-debugging-port={CDP_PORT}"))
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    // Discord opens the TCP port before the renderer answers `/json`; a probe can spend the
    // whole socket timeout on a half-started client. Cap the total wait instead of the count.
    let deadline = Instant::now() + Duration::from_secs(25);
    while Instant::now() < deadline {
        forget_refusal();
        if debug_port_client() == Some(picked) {
            return Ok(picked);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(format!("{} n'a pas ouvert le port de debug", picked.label()))
}

/// Whether a Discord executable is running.
#[cfg(windows)]
pub fn discord_running(exe: &str) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let output = std::process::Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {exe}"), "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    output.is_ok_and(|output| {
        String::from_utf8_lossy(&output.stdout)
            .to_ascii_lowercase()
            .contains(&exe.to_ascii_lowercase())
    })
}

#[cfg(not(windows))]
pub fn discord_running(_exe: &str) -> bool {
    false
}

#[cfg(not(windows))]
pub fn restart_with_debug_port() -> Result<Client, String> {
    Err("la relance avec le port de debug n'existe que sous Windows".to_owned())
}

/// A short line for the log: which transports answer right now.
pub fn describe() -> String {
    let cdp = match cdp_page() {
        Ok(_) => "port de debug ouvert".to_owned(),
        Err(error) => error,
    };
    let pipes = present_pipes();
    let rpc = if pipes.is_empty() {
        "aucun pipe Discord".to_owned()
    } else {
        pipes
            .iter()
            .map(|pipe| format!("discord-ipc-{pipe}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!("{cdp} · {rpc}")
}

/// The `discord-ipc-N` pipes that exist right now, listed without connecting to them.
pub fn present_pipes() -> Vec<u8> {
    #[cfg(windows)]
    {
        let mut found: Vec<u8> = std::fs::read_dir(r"\\.\pipe\")
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        entry
                            .file_name()
                            .to_str()?
                            .strip_prefix("discord-ipc-")?
                            .parse()
                            .ok()
                    })
                    .collect()
            })
            .unwrap_or_default();
        found.sort_unstable();
        found.dedup();
        found
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// One Discord pipe: which client serves it, and the account behind it when known.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PipeInfo {
    pub pipe: u8,
    pub client: Client,
    /// The account's display name, once a handshake told it.
    pub user: Option<String>,
}

/// What each Discord process answered once: its client and account, by process id. A process
/// keeps its account until it restarts (new id), so one handshake per Discord launch is enough.
static ACCOUNTS: Mutex<Vec<(u32, Client, String)>> = Mutex::new(Vec::new());

/// The Discord client serving a pipe, read from the pipe's server process: no handshake, so it
/// costs nothing against Discord's connection limit (see [`pipe_connect`]).
#[cfg(windows)]
fn pipe_owner(index: u8) -> Option<(u32, Client)> {
    use std::os::windows::io::AsRawHandle;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetNamedPipeServerProcessId(pipe: *mut core::ffi::c_void, pid: *mut u32) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut core::ffi::c_void;
        fn QueryFullProcessImageNameW(
            process: *mut core::ffi::c_void,
            flags: u32,
            name: *mut u16,
            size: *mut u32,
        ) -> i32;
        fn CloseHandle(handle: *mut core::ffi::c_void) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    let stream = open_pipe(index)?;
    let mut pid = 0u32;
    // SAFETY: the pipe handle lives as long as `stream`; the process handle is closed below.
    let exe = unsafe {
        if GetNamedPipeServerProcessId(stream.as_raw_handle(), &mut pid) == 0 {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut name = [0u16; 1024];
        let mut size = name.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, name.as_mut_ptr(), &mut size);
        CloseHandle(process);
        if ok == 0 {
            return None;
        }
        String::from_utf16_lossy(&name[..size as usize])
    };
    let exe = exe
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let client = match exe.as_str() {
        "discordptb.exe" => Client::Ptb,
        "discordcanary.exe" => Client::Canary,
        "discord.exe" => Client::Stable,
        _ => return None,
    };
    Some((pid, client))
}

/// Opens `discord-ipc-N`. Right after another program left it, Discord may not have put a
/// fresh instance up yet (the pipe answers "busy"): wait a little and try again.
#[cfg(windows)]
fn open_pipe(index: u8) -> Option<std::fs::File> {
    const ERROR_PIPE_BUSY: i32 = 231;
    let path = format!(r"\\.\pipe\discord-ipc-{index}");
    for _ in 0..4 {
        match std::fs::OpenOptions::new().read(true).write(true).open(&path) {
            Ok(stream) => return Some(stream),
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                std::thread::sleep(Duration::from_millis(60));
            }
            Err(_) => return None,
        }
    }
    None
}

/// Every Discord pipe, the client serving it, and its account when a handshake already told
/// it. Never shakes hands: safe to call often.
pub fn clients() -> Vec<PipeInfo> {
    #[cfg(windows)]
    {
        let known = ACCOUNTS.lock().map(|known| known.clone()).unwrap_or_default();
        present_pipes()
            .into_iter()
            .filter_map(|pipe| {
                let (pid, client) = pipe_owner(pipe)?;
                let user = known
                    .iter()
                    .find(|(id, _, _)| *id == pid)
                    .map(|(_, _, user)| user.clone());
                Some(PipeInfo { pipe, client, user })
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Every Discord pipe with its account, shaking hands (as `app_id`) only with the Discord
/// processes not met yet. Used by the diagnostic: pick an `app_id` the cards do not use, as
/// Discord limits the connections of each application.
pub fn accounts(app_id: u64) -> Result<Vec<PipeInfo>, String> {
    #[cfg(windows)]
    {
        let mut found = clients();
        let mut refused = None;
        for info in found.iter_mut().filter(|info| info.user.is_none()) {
            let Some((pid, _)) = pipe_owner(info.pipe) else {
                continue;
            };
            let Some(mut stream) = open_pipe(info.pipe) else {
                continue;
            };
            match handshake(&mut stream, app_id) {
                Ok(ready) => {
                    let user = user_name(&ready);
                    remember_account(pid, info.client, &user);
                    info.user = Some(user);
                }
                Err(error) => refused = Some(error),
            }
        }
        if !found.is_empty()
            && found.iter().all(|info| info.user.is_none())
            && let Some(error) = refused
        {
            return Err(error);
        }
        Ok(found)
    }
    #[cfg(not(windows))]
    {
        let _ = app_id;
        Err("le pipe Discord n'existe que sous Windows".to_owned())
    }
}

fn remember_account(pid: u32, client: Client, user: &str) {
    if let Ok(mut known) = ACCOUNTS.lock() {
        known.retain(|(id, _, _)| *id != pid);
        known.push((pid, client, user.to_owned()));
    }
}

/// The applications already looked up this session: their name, or `None` when Discord does
/// not know the ID.
static NAMES: Mutex<Option<HashMap<u64, Option<String>>>> = Mutex::new(None);

/// The application's name when it was already looked up this session; never waits.
pub fn known_application_name(app_id: u64) -> Option<String> {
    NAMES
        .lock()
        .ok()?
        .as_ref()?
        .get(&app_id)
        .cloned()
        .flatten()
}

/// Looks an application up on Discord's public API (no token): `Ok(Some(name))` when it
/// exists, `Ok(None)` when Discord does not know the ID, `Err` when it could not ask.
///
/// The renderer shows a card's `name` as it is (the pipe fills it from the application
/// itself), so a debug-port card with no name of its own takes the application's. The answer
/// is kept for the session; `curl` ships with Windows 10 and 11.
pub fn application_name(app_id: u64) -> Result<Option<String>, String> {
    if let Ok(names) = NAMES.lock()
        && let Some(known) = names.as_ref().and_then(|names| names.get(&app_id))
    {
        return Ok(known.clone());
    }
    let url = format!("https://discord.com/api/v10/applications/{app_id}/rpc");
    let mut command = std::process::Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
    command.args(["-s", "--max-time", "6", "-w", "\n%{http_code}", &url]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command
        .output()
        .map_err(|error| format!("curl introuvable ({error})"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", text.as_ref()));
    let found = match code.trim() {
        "200" => Some(
            serde_json::from_str::<Value>(body)
                .ok()
                .and_then(|app| app.get("name").and_then(Value::as_str).map(str::to_owned))
                .ok_or("réponse de Discord illisible")?,
        ),
        // Unknown application, or not even a valid ID.
        "404" | "400" => None,
        "" | "000" => return Err("pas de connexion à Internet".to_owned()),
        other => return Err(format!("Discord a répondu {other}")),
    };
    if let Ok(mut names) = NAMES.lock() {
        names
            .get_or_insert_with(HashMap::new)
            .insert(app_id, found.clone());
    }
    Ok(found)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// The card's start time, kept while the same card is pushed again.
///
/// A Listening card also needs an end: Discord draws the playback bar from the two
/// timestamps, and a track that never ends keeps the bar stuck at 100%. When the span
/// is over the start moves forward, so the bar loops instead of freezing.
fn start_for(key: &str, span: Option<u64>) -> u64 {
    static START: OnceLock<Mutex<(String, u64)>> = OnceLock::new();
    let cell = START.get_or_init(|| Mutex::new((String::new(), now_ms())));
    let mut kept = match cell.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let now = now_ms();
    if kept.0 != key || span.is_some_and(|span| now >= kept.1.saturating_add(span)) {
        *kept = (key.to_owned(), now);
    }
    kept.1
}

/// A text cut to Discord's `max`, and padded when it is a single character (the pipe refuses
/// those) with an invisible filler instead of failing the whole card.
fn fit(text: &str, max: usize) -> String {
    let text = text.trim();
    match text.chars().count() {
        1 => format!("{text}{FILLER}"),
        count if count > max => {
            let mut cut: String = text.chars().take(max - 1).collect();
            cut.push('…');
            cut
        }
        _ => text.to_owned(),
    }
}

/// A button label cut to Discord's 32 characters (one character is fine for a label).
fn label(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() > LABEL_MAX {
        text.chars().take(LABEL_MAX).collect()
    } else {
        text.to_owned()
    }
}

/// How the pipe and the renderer spell the two buttons.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ButtonStyle {
    /// The debug-port event: labels in `buttons`, targets in `metadata.button_urls`.
    Cdp,
    /// The IPC pipe: `[{ "label", "url" }]`; Discord rewrites the couple itself.
    Rpc,
}

/// The activity object both transports send.
fn activity(config: &Config, image: &str, small: &str, buttons: ButtonStyle) -> Value {
    // The app id, not the user-visible title: renaming a Spotify card must not lose the
    // Spotify fields that draw its logo and playback bar.
    let spotify = config.kind == 2 && config.app_id == SPOTIFY_APP_ID;
    let mut value = json!({
        "application_id": config.app_id.to_string(),
        "type": config.kind,
        "instance": true,
        // Discord's renderer only keeps a local activity carrying the "instance" flag.
        // Vencord sets the same bit (1 << 0); without it the CDP event is silently dropped.
        // Spotify's own card also carries SYNC|PLAY (48), which is what its renderer
        // recognises for a Listening card.
        "flags": if spotify { 48 } else { 1 },
    });
    // An empty name is left out rather than sent empty: the pipe then uses the application's.
    if !config.name.trim().is_empty() {
        value["name"] = json!(fit(&config.name, TEXT_MAX));
    }
    if !config.details.trim().is_empty() {
        value["details"] = json!(fit(&config.details, TEXT_MAX));
    }
    if !config.state.trim().is_empty() {
        value["state"] = json!(fit(&config.state, TEXT_MAX));
    }
    if !image.is_empty() || !small.is_empty() {
        let mut assets = json!({});
        if !image.is_empty() {
            assets["large_image"] = json!(image);
            if !config.image_text.trim().is_empty() {
                assets["large_text"] = json!(fit(&config.image_text, TEXT_MAX));
            }
        }
        if !small.is_empty() {
            assets["small_image"] = json!(small);
            if !config.small_text.trim().is_empty() {
                assets["small_text"] = json!(fit(&config.small_text, TEXT_MAX));
            }
        }
        value["assets"] = assets;
    }
    if config.timer || spotify {
        let span = (config.kind == 2).then_some(LISTEN_MILLIS);
        let start = start_for(&config.signature(), span);
        value["timestamps"] = if config.kind == 2 {
            json!({ "start": start, "end": start + LISTEN_MILLIS })
        } else {
            json!({ "start": start })
        };
    }
    if spotify {
        // The shape Discord's own Spotify integration reads: a Listening activity with a
        // track id, a Spotify party and track metadata. Without them the card fell back to
        // the generic music note and dropped the playback bar.
        value["sync_id"] = json!("4vDCBZVBloslnt43x3opk3");
        value["party"] = json!({ "id": "spotify:spider" });
        let mut metadata = value
            .get("metadata")
            .cloned()
            .unwrap_or_else(|| json!({}));
        metadata["type"] = json!("track");
        if metadata.get("button_urls").is_none() {
            metadata["button_urls"] = json!([]);
        }
        value["metadata"] = metadata;
    }
    if config.kind == 1 {
        let url = if config.stream_url.is_empty() {
            "https://www.twitch.tv/"
        } else {
            &config.stream_url
        };
        value["url"] = json!(url);
    }
    if let Some(platform) = config.platform {
        value["platform"] = json!(platform);
    }
    // A link longer than Discord accepts would fail the whole card: leave that button out.
    let links: Vec<(String, &str)> = config
        .buttons
        .iter()
        .filter(|(_, url)| url.len() <= URL_MAX)
        .map(|(text, url)| (label(text), url.as_str()))
        .filter(|(text, _)| !text.is_empty())
        .take(2)
        .collect();
    if !links.is_empty() {
        match buttons {
            ButtonStyle::Cdp => {
                // Vencord's shape: the renderer takes the labels and reads the targets
                // from `metadata.button_urls`.
                let labels: Vec<Value> = links.iter().map(|(text, _)| json!(text)).collect();
                let urls: Vec<Value> = links.iter().map(|(_, url)| json!(url)).collect();
                value["buttons"] = Value::Array(labels);
                value["metadata"] = json!({ "button_urls": urls });
            }
            ButtonStyle::Rpc => {
                // The pipe validates each button as an object; sending only the labels
                // fails the whole SET_ACTIVITY with code 4000 (and takes the card down).
                let objects: Vec<Value> = links
                    .iter()
                    .map(|(text, url)| json!({ "label": text, "url": url }))
                    .collect();
                value["buttons"] = Value::Array(objects);
            }
        }
    }
    value
}

// --- Debug port (CDP) -------------------------------------------------------

fn cdp_push(config: &Config) -> Result<(), String> {
    let signer = if config.image_signer == 0 {
        config.app_id
    } else {
        config.image_signer
    };
    // The pipe names the card after its application; the renderer shows `name` as it is, so a
    // card with no name of its own would have an empty title there.
    let mut config = config.clone();
    if config.name.trim().is_empty()
        && let Ok(Some(name)) = application_name(config.app_id)
    {
        config.name = name;
    }
    let (image, small) = proxied_images(signer, &config.image, &config.small_image);
    let value = activity(&config, &image, &small, ButtonStyle::Cdp);
    let socket = format!("\"{}\"", SOCKETS[usize::from(config.slot.clamp(1, 3)) - 1]);
    let script = format!(
        "window.__spiderActivity = {value};\n{}",
        DISPATCH_JS
            .replace("__ACTIVITY__", "window.__spiderActivity")
            .replace("__SOCKET__", &socket)
    );
    let verdict = cdp_eval(&script)?;
    if verdict.get("ok").and_then(Value::as_bool) != Some(true) {
        let error = verdict
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("pas de dispatcher Flux dans le renderer");
        return Err(format!("port de debug : {error}"));
    }
    // A dispatch that reached no store looks exactly like a successful one from the socket
    // side: without this check the card silently stayed empty and the RPC fallback never ran.
    if verdict.get("found").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    // Discord's store can take a tick to show the card. The old code called one slow tick a
    // failure and switched to the pipe, which put a second card of another type on the
    // profile; look a few times first.
    let card = CARD_THERE_JS
        .replace("__APP__", &config.app_id.to_string())
        .replace(
            "\"__NAME__\"",
            &serde_json::to_string(&fit(&config.name, TEXT_MAX)).unwrap_or_default(),
        );
    for _ in 0..6 {
        std::thread::sleep(Duration::from_millis(40));
        if matches!(cdp_eval(&card), Ok(Value::Bool(true))) {
            return Ok(());
        }
    }
    // The cached dispatcher may belong to an old renderer state: forget it and send once
    // more, so the next refresh finds a fresh one.
    let _ = cdp_eval(FORGET_FLUX_JS);
    let verdict = cdp_eval(&script)?;
    if verdict.get("found").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    Err("port de debug : Discord n'a gardé aucune activité".to_owned())
}

fn cdp_clear() -> Result<(), String> {
    let mut last = String::new();
    let mut cleared = false;
    // Each socket id is one local activity: the three slots, then the old tool's names.
    for socket in SOCKETS.iter().chain(OLD_SOCKETS.iter()) {
        let socket = format!("\"{socket}\"");
        let js = DISPATCH_JS
            .replace("__ACTIVITY__", "null")
            .replace("__SOCKET__", &socket);
        match cdp_eval(&js) {
            Ok(verdict) if verdict.get("ok").and_then(Value::as_bool) == Some(true) => {
                cleared = true;
            }
            Ok(_) => {}
            Err(error) => {
                last = error;
                // A closed port answers every socket the same way: no need to ask five more.
                break;
            }
        }
    }
    if cleared {
        Ok(())
    } else if last.is_empty() {
        Err("port de debug : Discord a gardé l'activité".to_owned())
    } else {
        Err(last)
    }
}

/// Sends `LOCAL_ACTIVITY_UPDATE` to Discord's real Flux dispatcher.
///
/// Discord wraps every module export in a proxy, and the 95 `IntlMessagesProxy` objects also
/// answer `dispatch`: calling those did nothing, yet the CDP call reported success, so the
/// old code cleared the pipe card and then published nothing. The Flux dispatcher is not an
/// export: every store keeps it in `_dispatcher`. Find one, dispatch once, then check that a
/// store actually kept the activity before claiming the push worked.
///
/// The found dispatcher is kept on `window.__spiderFlux` and reused: this used to walk
/// Discord's whole module cache on every refresh (three cards, every five seconds), which
/// made the renderer janky on a big client and could take it down.
const DISPATCH_JS: &str = r#"(() => {
  const result = { ok: false, dispatcher: "", found: false, error: "" };
  const safe = (fn, fallback) => { try { return fn(); } catch (err) { return fallback; } };
  const valid = (flux) => {
    if (!flux) return false;
    const d = safe(() => flux.dispatcher, null);
    if (!d || typeof d.dispatch !== "function" || typeof d.subscribe !== "function") return false;
    const stores = safe(() => flux.stores, null);
    return Array.isArray(stores) && stores.length > 0;
  };
  let flux = valid(window.__spiderFlux) ? window.__spiderFlux : null;
  if (!flux) {
    if (!window.webpackChunkdiscord_app) {
      result.error = "Discord est encore en train de charger";
      return result;
    }
    let req = null;
    window.webpackChunkdiscord_app.push([[Math.random()], {}, (e) => {
      if (e && e.c && !req) req = e;
    }]);
    if (!req) {
      result.error = "le cache de modules de Discord n'est pas prêt";
      return result;
    }
    const found = { dispatcher: null, stores: [] };
    const seen = new Set();
    const candidates = (exp) => {
      const list = [exp];
      const def = safe(() => exp.default, null);
      if (def) list.push(def);
      for (const key of safe(() => Object.keys(exp), []) || []) {
        const value = safe(() => exp[key], null);
        if (value && typeof value === "object") list.push(value);
      }
      return list;
    };
    for (const id of safe(() => Object.keys(req.c), []) || []) {
      const mod = safe(() => req.c[id], null);
      if (!mod) continue;
      const exp = safe(() => mod.exports, null);
      if (!exp) continue;
      for (const c of candidates(exp)) {
        if (!c || (typeof c !== "object" && typeof c !== "function") || seen.has(c)) continue;
        seen.add(c);
        // The i18n message proxies answer every name, `dispatch` and `_dispatcher` included.
        if (safe(() => c[Symbol.toStringTag], "") === "IntlMessagesProxy" || safe(() => c.$$loader, null)) continue;
        const d = safe(() => c._dispatcher, null);
        if (d && typeof d.dispatch === "function" && typeof d.subscribe === "function") {
          if (!found.dispatcher) found.dispatcher = d;
          if (typeof c.getActivities === "function") found.stores.push(c);
        }
      }
    }
    if (!found.dispatcher || found.stores.length === 0) {
      result.error = found.dispatcher ? "Discord n'a pas de store d'activités" : "pas de dispatcher Flux";
      return result;
    }
    flux = found;
    window.__spiderFlux = flux;
    result.dispatcher = "scanned";
  } else {
    result.dispatcher = "cached";
  }
  const activity = __ACTIVITY__;
  try {
    flux.dispatcher.dispatch({
      type: "LOCAL_ACTIVITY_UPDATE",
      activity,
      socketId: __SOCKET__,
    });
    result.ok = true;
  } catch (err) {
    result.error = String(err);
    return result;
  }
  if (activity == null) {
    result.found = true;
    return result;
  }
  const wantedApp = String(activity.application_id || "");
  const wantedName = String(activity.name || "");
  for (const store of flux.stores) {
    const list = safe(() => store.getActivities(), null);
    if (!Array.isArray(list)) continue;
    if (list.some((a) => a && (String(a.application_id || "") === wantedApp || (wantedName && a.name === wantedName)))) {
      result.found = true;
      break;
    }
  }
  return result;
})()"#;

/// Whether the renderer's activity store holds the card now (the dispatch can take a tick).
const CARD_THERE_JS: &str = r#"(() => {
  const flux = window.__spiderFlux;
  if (!flux || !Array.isArray(flux.stores)) return false;
  const wantedApp = "__APP__";
  const wantedName = "__NAME__";
  return flux.stores.some((store) => {
    try {
      const list = store.getActivities();
      return Array.isArray(list) && list.some((a) => a && (String(a.application_id || "") === wantedApp || (wantedName && a.name === wantedName)));
    } catch (err) {
      return false;
    }
  });
})()"#;

/// Drops the cached dispatcher, so the next dispatch finds a fresh one.
const FORGET_FLUX_JS: &str = "(() => { delete window.__spiderFlux; return true; })()";

/// The host of a Discord main-window page, or `None` for anything else (other sites, the
/// in-game overlay, popouts): the activity must reach the main renderer's stores.
fn main_window_host(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = authority.split(':').next()?;
    let discord = host == "discord.com"
        || host.ends_with(".discord.com")
        || host == "discordapp.com"
        || host.ends_with(".discordapp.com");
    let side_window = path.starts_with("overlay") || path.contains("popout");
    (discord && !side_window).then_some(host)
}

/// The renderer's WebSocket: its host, port and path.
fn cdp_page() -> Result<(String, u16, String), String> {
    let wanted = client();
    let body = http_get(CDP_PORT, "/json")?;
    let pages: Value = serde_json::from_str(&body).map_err(|error| error.to_string())?;
    let mut other = None;
    for page in pages.as_array().into_iter().flatten() {
        if page
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind != "page")
        {
            continue;
        }
        let url = page.get("url").and_then(Value::as_str).unwrap_or_default();
        let Some(host) = main_window_host(url) else {
            continue;
        };
        let found = Client::of_host(host);
        if wanted != Client::Auto && found != wanted {
            other = Some(found);
            continue;
        }
        let socket = page
            .get("webSocketDebuggerUrl")
            .and_then(Value::as_str)
            .ok_or("port de debug : pas de websocket pour la page Discord")?;
        if let Some(split) = split_websocket(socket) {
            return Ok(split);
        }
    }
    Err(match other {
        Some(found) => format!(
            "port de debug : il est ouvert par {}, pas par {}",
            found.label(),
            wanted.label()
        ),
        None => "port de debug : Discord n'est pas lancé avec --remote-debugging-port".to_owned(),
    })
}

fn split_websocket(socket: &str) -> Option<(String, u16, String)> {
    let rest = socket.strip_prefix("ws://")?;
    let (authority, path) = rest.split_once('/')?;
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host.to_owned(), port.parse().ok()?),
        None => (authority.to_owned(), 80),
    };
    Some((host, port, format!("/{path}")))
}

/// When the debug port last refused a connection.
///
/// Windows takes about two seconds to report a closed local port, and a push asked it up to
/// three times (then the clear four more): with Discord started normally, every failed push
/// took six to ten seconds. A refusal is now trusted for a few seconds.
static REFUSED_AT: Mutex<Option<Instant>> = Mutex::new(None);
const REFUSED_FOR: Duration = Duration::from_secs(4);

/// Forgets a recent refusal, to see the port as soon as Discord opens it.
fn forget_refusal() {
    if let Ok(mut at) = REFUSED_AT.lock() {
        *at = None;
    }
}

fn http_get(port: u16, path: &str) -> Result<String, String> {
    const CLOSED: &str = "port de debug : fermé";
    if REFUSED_AT
        .lock()
        .ok()
        .and_then(|at| *at)
        .is_some_and(|at| at.elapsed() < REFUSED_FOR)
    {
        return Err(CLOSED.to_owned());
    }
    let mut stream = match TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        TIMEOUT,
    ) {
        Ok(stream) => stream,
        Err(_) => {
            if let Ok(mut at) = REFUSED_AT.lock() {
                *at = Some(Instant::now());
            }
            return Err(CLOSED.to_owned());
        }
    };
    stream.set_read_timeout(Some(TIMEOUT)).ok();
    stream.set_write_timeout(Some(TIMEOUT)).ok();
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| error.to_string())?;
    // Chrome does not always close a `Connection: close` reply at once, and `read_to_end`
    // turns that into a timeout: read chunks and stop at EOF, at the announced length, or
    // after the timeout once a body is in hand. `cdp_page` was the reason CDP mode never
    // worked: the HTTP probe itself timed out before any websocket was opened.
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                raw.extend_from_slice(&chunk[..read]);
                if let Some((head, _)) = String::from_utf8_lossy(&raw).split_once("\r\n\r\n")
                    && let Some(length) = content_length(head)
                    && raw.len() >= head.len() + 4 + length
                {
                    break;
                }
                // A reply far larger than any page list is not Discord's.
                if raw.len() > 4 * 1024 * 1024 {
                    return Err("port de debug : réponse trop grande".to_owned());
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    let text = String::from_utf8_lossy(&raw);
    let (_, body) = text
        .split_once("\r\n\r\n")
        .ok_or("port de debug : réponse HTTP illisible")?;
    Ok(body.to_owned())
}

/// `Content-Length` of an HTTP reply head, when it announces one.
fn content_length(head: &str) -> Option<usize> {
    head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse().ok())
            .flatten()
    })
}

fn cdp_eval(expression: &str) -> Result<Value, String> {
    let (host, port, path) = cdp_page()?;
    let mut stream = TcpStream::connect((host.as_str(), port)).map_err(|error| error.to_string())?;
    stream.set_read_timeout(Some(TIMEOUT)).ok();
    stream.set_write_timeout(Some(TIMEOUT)).ok();
    let handshake = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    stream
        .write_all(handshake.as_bytes())
        .map_err(|error| error.to_string())?;
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream
            .read_exact(&mut byte)
            .map_err(|error| error.to_string())?;
        head.push(byte[0]);
        if head.len() > 8192 {
            return Err("port de debug : poignée de main websocket trop longue".to_owned());
        }
    }
    if !head.starts_with(b"HTTP/1.1 101") {
        return Err("port de debug : websocket refusée".to_owned());
    }
    let request = json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "params": { "expression": expression, "returnByValue": true },
    });
    send_text(&mut stream, request.to_string().as_bytes())?;
    for _ in 0..16 {
        let frame = read_frame(&mut stream)?;
        if frame.is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_slice(&frame) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value.get("id").and_then(Value::as_u64) != Some(1) {
            continue;
        }
        if let Some(exception) = value.pointer("/result/exceptionDetails") {
            return Err(format!("port de debug : {exception}"));
        }
        let value = value
            .pointer("/result/result/value")
            .cloned()
            .unwrap_or(Value::Null);
        return Ok(value);
    }
    Err("port de debug : le renderer n'a pas répondu".to_owned())
}

fn send_text(stream: &mut TcpStream, payload: &[u8]) -> Result<(), String> {
    let mask = [0x12u8, 0x34, 0x56, 0x78];
    let mut frame = Vec::with_capacity(payload.len() + 14);
    frame.push(0x81); // FIN + text
    if payload.len() < 126 {
        frame.push(0x80 | payload.len() as u8);
    } else if payload.len() <= usize::from(u16::MAX) {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4]),
    );
    stream.write_all(&frame).map_err(|error| error.to_string())
}

fn read_frame(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    let mut head = [0u8; 2];
    stream
        .read_exact(&mut head)
        .map_err(|error| error.to_string())?;
    let opcode = head[0] & 0x0f;
    let mut length = usize::from(head[1] & 0x7f);
    if length == 126 {
        let mut wide = [0u8; 2];
        stream
            .read_exact(&mut wide)
            .map_err(|error| error.to_string())?;
        length = usize::from(u16::from_be_bytes(wide));
    } else if length == 127 {
        let mut wide = [0u8; 8];
        stream
            .read_exact(&mut wide)
            .map_err(|error| error.to_string())?;
        length = u64::from_be_bytes(wide) as usize;
    }
    // A garbled length must not ask for gigabytes.
    if length > 8 * 1024 * 1024 {
        return Err("port de debug : trame trop grande".to_owned());
    }
    // Servers never mask their frames.
    let mut data = vec![0u8; length];
    stream
        .read_exact(&mut data)
        .map_err(|error| error.to_string())?;
    // Ping and close frames carry nothing for us.
    if opcode == 0x9 || opcode == 0x8 {
        return Ok(Vec::new());
    }
    Ok(data)
}

/// Signed picture paths, by generation, application and pictures.
type SignedCache = HashMap<String, (String, String, Instant)>;

/// Discord only renders `mp:external/...`: the main process signs that path, so
/// ask it once through the pipe and keep the answer.
fn proxied_images(app_id: u64, large: &str, small: &str) -> (String, String) {
    let external = |image: &str| image.starts_with("http://") || image.starts_with("https://");
    if !external(large) && !external(small) {
        return (large.to_owned(), small.to_owned());
    }
    // The generation only changes when Discord is restarted (the pipes are reconnected), not
    // when another application opens its own pipe: keying on every connection threw the
    // signed paths away all the time, and each re-sign put a blank card on the profile and
    // cleared it again, which made the real card blink.
    let key = format!(
        "{}|{app_id}|{large}|{small}",
        SIGN_GENERATION.load(Ordering::Relaxed)
    );
    static CACHE: OnceLock<Mutex<SignedCache>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(known) = cache.lock()
        && let Some((signed_large, signed_small, at)) = known.get(&key)
        && at.elapsed() < SIGNED_FOR
    {
        return (signed_large.clone(), signed_small.clone());
    }
    // One temporary card signs both pictures. The pipe is the only public way to ask the
    // main process for the `mp:external/...` path the renderer renders.
    let mut assets = json!({});
    if external(large) {
        assets["large_image"] = json!(large);
        assets["large_text"] = json!("...");
    }
    if external(small) {
        assets["small_image"] = json!(small);
        assets["small_text"] = json!("...");
    }
    let temp = json!({
        "details": "...",
        "state": "...",
        "assets": assets,
        "instance": true,
    });
    let reply = pipe_request(
        app_id,
        "SET_ACTIVITY",
        json!({ "pid": std::process::id(), "activity": temp }),
    );
    // Remove the blank card at once: it is not the user's activity.
    let _ = pipe_request(
        app_id,
        "SET_ACTIVITY",
        json!({ "pid": std::process::id(), "activity": Value::Null }),
    );
    let signed = reply.ok().map(|reply| {
        let get = |path: &str| {
            reply
                .pointer(path)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        (
            get("/data/assets/large_image").unwrap_or_else(|| large.to_owned()),
            get("/data/assets/small_image").unwrap_or_else(|| small.to_owned()),
        )
    });
    let (signed_large, signed_small) =
        signed.unwrap_or_else(|| (large.to_owned(), small.to_owned()));
    if let Ok(mut known) = cache.lock() {
        known.insert(
            key,
            (signed_large.clone(), signed_small.clone(), Instant::now()),
        );
    }
    (signed_large, signed_small)
}

// --- Local pipe -------------------------------------------------------------

fn rpc_push(config: &Config) -> Result<(), String> {
    // The pipe signs `https://...` itself and refuses the already-signed `mp:external/`
    // paths (it drops the picture silently). Send the raw URLs; only CDP needs the signed
    // form.
    let mut value = activity(config, &config.image, &config.small_image, ButtonStyle::Rpc);
    // The pipe refuses type 1 and drops the platform: fall back to Playing.
    if config.kind == 1 {
        value["type"] = json!(0);
        if let Some(map) = value.as_object_mut() {
            map.remove("url");
        }
    }
    if let Some(map) = value.as_object_mut() {
        map.remove("platform");
    }
    pipe_request(
        config.app_id,
        "SET_ACTIVITY",
        json!({ "pid": std::process::id(), "activity": value }),
    )
    .map(|_| ())
}

fn rpc_clear() -> Result<(), String> {
    #[cfg(windows)]
    {
        // Clearing on every connected pipe; Discord drops an activity with its pipe.
        let mut open = pipes().lock().map_err(|error| error.to_string())?;
        let mut closed = 0;
        for (_, mut pipe) in open.drain() {
            closed += 1;
            let payload = json!({
                "cmd": "SET_ACTIVITY",
                "args": { "pid": std::process::id(), "activity": Value::Null },
                "nonce": "clear",
            })
            .to_string();
            let _ = pipe_write(&mut pipe.stream, 1, payload.as_bytes());
            let _ = pipe_read(&mut pipe.stream);
        }
        // No pipe is left: whatever Discord signed for them must be asked again. With no pipe
        // to close, nothing was signed for this generation in the first place, and bumping
        // anyway threw away the valid `mp:external/...` paths on every transport switch.
        if closed > 0 {
            SIGN_GENERATION.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err("le pipe Discord n'existe que sous Windows".to_owned())
    }
}

/// Sends one command on Discord's IPC pipe and returns its reply.
fn pipe_request(app_id: u64, command: &str, args: Value) -> Result<Value, String> {
    #[cfg(windows)]
    {
        pipe_request_windows(app_id, command, args)
    }
    #[cfg(not(windows))]
    {
        let _ = (app_id, command, args);
        Err("le pipe Discord n'existe que sous Windows".to_owned())
    }
}

/// Discord ties an activity to the IPC connection that set it: closing the pipe after the
/// handshake made the card flash then vanish. The connection is kept open and reused.
#[cfg(windows)]
struct Pipe {
    stream: std::fs::File,
    nonce: u32,
}

#[cfg(windows)]
static PIPES: OnceLock<Mutex<HashMap<u64, Pipe>>> = OnceLock::new();

#[cfg(windows)]
fn pipes() -> &'static Mutex<HashMap<u64, Pipe>> {
    PIPES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Discord's words for a refused connection, in the tool's language.
fn refusal(message: &str) -> String {
    match message {
        "Invalid Client ID" => "ID d'application inconnu de Discord".to_owned(),
        "Invalid Origin" => "origine refusée par Discord".to_owned(),
        other => other.to_owned(),
    }
}

/// Opens Discord's IPC session on `stream`: the READY event, or Discord's refusal.
#[cfg(windows)]
fn handshake(stream: &mut std::fs::File, app_id: u64) -> Result<Value, String> {
    pipe_write(
        stream,
        0,
        json!({ "v": 1, "client_id": app_id.to_string() })
            .to_string()
            .as_bytes(),
    )?;
    let (opcode, reply) = pipe_read(stream)?;
    // A refused handshake comes back as a CLOSE frame (`{code, message}`), not as READY.
    if opcode == 2 || reply.get("evt").and_then(Value::as_str) != Some("READY") {
        let field = |key: &str| {
            reply
                .get(key)
                .or_else(|| reply.pointer(&format!("/data/{key}")))
                .cloned()
        };
        let message = field("message")
            .and_then(|message| message.as_str().map(refusal))
            .unwrap_or_else(|| "Discord a refusé la connexion".to_owned());
        let code = field("code").and_then(|code| code.as_u64()).unwrap_or(0);
        return Err(format!("Discord IPC : {message} (code {code})"));
    }
    Ok(reply)
}

/// The client the cards last reached in Auto, so a later reconnection goes back to the same
/// account instead of whichever pipe answers first.
static LAST_CLIENT: AtomicU8 = AtomicU8::new(0);

/// What a silent Discord means, in the words the window shows.
const SILENT: &str =
    "Discord ne répond plus : il ignore un moment les connexions trop rapprochées, nouvel essai bientôt";

/// Opens the pipe of the chosen client and shakes hands as `app_id`.
///
/// Discord only answers about two handshakes of one application in a short while, then stays
/// silent for about a minute (measured: the third quick connection gets no READY at all). So
/// this connects only to the pipe it means to use (each pipe's client is read from its server
/// process first), the connection is kept for the whole session, and a silent Discord is
/// reported instead of trying the next pipe, which may be another account.
#[cfg(windows)]
fn pipe_connect(app_id: u64) -> Result<Pipe, String> {
    let wanted = client();
    let pipes = clients();
    let mut candidates: Vec<u8> = match wanted {
        Client::Auto => {
            let last = Client::from_code(LAST_CLIENT.load(Ordering::Relaxed));
            let mut order: Vec<&PipeInfo> = pipes.iter().collect();
            order.sort_by_key(|info| (info.client != last, info.pipe));
            order.into_iter().map(|info| info.pipe).collect()
        }
        chosen => pipes
            .iter()
            .filter(|info| info.client == chosen)
            .map(|info| info.pipe)
            .collect(),
    };
    // Owners unreadable (an unusual Discord build): every pipe, in order.
    if pipes.is_empty() {
        candidates = present_pipes();
    }
    if candidates.is_empty() {
        return Err(if present_pipes().is_empty() {
            "Discord n'est pas ouvert".to_owned()
        } else {
            format!("{} n'est pas ouvert", wanted.label())
        });
    }
    let mut last = String::from("Discord n'est pas ouvert");
    for index in candidates {
        let Some(mut stream) = open_pipe(index) else {
            continue;
        };
        let ready = match handshake(&mut stream, app_id) {
            Ok(ready) => ready,
            // Silence is Discord's connection limit, not a missing client: wait rather than
            // hop to another pipe (and maybe another account).
            Err(error) if error.contains(NO_ANSWER) => return Err(SILENT.to_owned()),
            Err(error) => {
                last = error;
                continue;
            }
        };
        let found = Client::of_ready(&ready);
        if wanted != Client::Auto && found != wanted {
            last = format!("{} n'est pas ouvert", wanted.label());
            continue;
        }
        let user = user_name(&ready);
        if let Some((pid, _)) = pipe_owner(index) {
            remember_account(pid, found, &user);
        }
        LAST_CLIENT.store(found.code(), Ordering::Relaxed);
        if let Ok(mut account) = ACCOUNT.lock() {
            *account = Some(Account {
                client: found,
                name: user,
                pipe: index,
            });
        }
        return Ok(Pipe { stream, nonce: 0 });
    }
    Err(last)
}

#[cfg(windows)]
fn pipe_send(pipe: &mut Pipe, command: &str, args: Value) -> Result<Value, String> {
    pipe.nonce = pipe.nonce.wrapping_add(1).max(1);
    let nonce = pipe.nonce.to_string();
    let payload = json!({ "cmd": command, "args": args, "nonce": nonce }).to_string();
    pipe_write(&mut pipe.stream, 1, payload.as_bytes())?;
    for _ in 0..8 {
        let (opcode, reply) = pipe_read(&mut pipe.stream)?;
        match opcode {
            // PING: answer with the same payload, then keep waiting for the reply.
            3 => {
                pipe_write(&mut pipe.stream, 4, reply.to_string().as_bytes())?;
                continue;
            }
            2 => {
                let message = reply
                    .get("message")
                    .and_then(Value::as_str)
                    .map(refusal)
                    .unwrap_or_else(|| "connexion fermée".to_owned());
                return Err(format!("Discord IPC : {message}"));
            }
            _ => {}
        }
        if reply.get("nonce").and_then(Value::as_str) == Some(nonce.as_str())
            || reply.get("cmd").and_then(Value::as_str) == Some(command)
        {
            // Discord answers a rejected SET_ACTIVITY with `evt: "ERROR"` and still exits 0:
            // reporting that as success left the profile empty and the game never retried.
            if reply.get("evt").and_then(Value::as_str) == Some("ERROR") {
                let message = reply
                    .pointer("/data/message")
                    .and_then(Value::as_str)
                    .unwrap_or("Discord a refusé l'activité");
                let code = reply
                    .pointer("/data/code")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                return Err(format!("Discord IPC : {message} (code {code})"));
            }
            return Ok(reply);
        }
    }
    Err("Discord IPC : pas de réponse à cette commande".to_owned())
}

#[cfg(windows)]
fn pipe_request_windows(app_id: u64, command: &str, args: Value) -> Result<Value, String> {
    let mut open = pipes().lock().map_err(|error| error.to_string())?;
    if let std::collections::hash_map::Entry::Vacant(slot) = open.entry(app_id) {
        slot.insert(pipe_connect(app_id)?);
    }
    let mut retry = true;
    loop {
        let result = {
            let pipe = open
                .get_mut(&app_id)
                .ok_or_else(|| "Discord IPC : pas de pipe pour cette application".to_owned())?;
            pipe_send(pipe, command, args.clone())
        };
        match result {
            Ok(reply) => return Ok(reply),
            Err(_) if retry => {
                retry = false;
                open.remove(&app_id);
                // Discord restarted or the pipe went away: the paths it signed are dead too.
                SIGN_GENERATION.fetch_add(1, Ordering::Relaxed);
                let pipe = pipe_connect(app_id)?;
                open.insert(app_id, pipe);
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(windows)]
fn pipe_write(stream: &mut std::fs::File, opcode: u32, payload: &[u8]) -> Result<(), String> {
    let mut frame = Vec::with_capacity(payload.len() + 8);
    frame.extend_from_slice(&opcode.to_le_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    stream
        .write_all(&frame)
        .map_err(|error| error.to_string())
}

/// How a pipe read that got no answer in time ends (Discord's connection limit looks like this).
const NO_ANSWER: &str = "Discord n'a pas répondu à temps";

/// Reads exactly `buffer` from the pipe, giving up after a deadline.
///
/// A bare `read_exact` on Discord's pipe blocks forever when Discord is busy, frozen, or
/// closed in an odd state. The worker held the shared pipe mutex while it waited, so every
/// later refresh queued behind it, threads piled up one per refresh, and the card stopped
/// following the settings (or the game hung). Peek first, then read only when the bytes are
/// really there.
#[cfg(windows)]
fn pipe_exact(stream: &mut std::fs::File, buffer: &mut [u8]) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn PeekNamedPipe(
            handle: *mut core::ffi::c_void,
            buffer: *mut core::ffi::c_void,
            size: u32,
            read: *mut u32,
            available: *mut u32,
            left: *mut u32,
        ) -> i32;
    }

    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        let mut count = 0u32;
        // SAFETY: `handle` is a live named pipe for as long as `stream` lives; null buffers
        // with size 0 ask only for the number of bytes waiting.
        let peeked = unsafe {
            PeekNamedPipe(
                stream.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut count,
                std::ptr::null_mut(),
            )
        };
        if peeked == 0 {
            return Err("Discord IPC : le pipe s'est fermé".to_owned());
        }
        // A whole frame is small, so wait for all of it and read once: a partial read on a
        // pipe blocks until the rest arrives anyway.
        if count as usize >= buffer.len() {
            stream
                .read_exact(buffer)
                .map_err(|error| error.to_string())?;
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Err(format!("Discord IPC : {NO_ANSWER}"))
}

/// One frame from the pipe: its opcode (0 handshake, 1 frame, 2 close, 3 ping, 4 pong) and
/// its JSON payload.
#[cfg(windows)]
fn pipe_read(stream: &mut std::fs::File) -> Result<(u32, Value), String> {
    let mut header = [0u8; 8];
    pipe_exact(stream, &mut header)?;
    let opcode = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    // A garbled length must not ask for gigabytes.
    if length > 1024 * 1024 {
        return Err("Discord IPC : réponse trop grande".to_owned());
    }
    let mut body = vec![0u8; length];
    pipe_exact(stream, &mut body)?;
    let value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    Ok((opcode, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> Config {
        Config {
            app_id: 1,
            image_signer: 2,
            name: "spider".into(),
            details: "spider".into(),
            image: "https://example.test/logo.png".into(),
            image_text: "spider".into(),
            kind: 0,
            timer: true,
            ..Config::default()
        }
    }

    #[test]
    fn the_timer_and_the_signer_are_part_of_the_signature() {
        let on = card();
        let mut off = on.clone();
        off.timer = false;
        assert_ne!(on.signature(), off.signature());
        let mut other = on.clone();
        other.image_signer = 3;
        assert_ne!(on.signature(), other.signature());
    }

    #[test]
    fn the_activity_carries_the_flags_discord_expects() {
        let mut c = card();
        c.buttons = vec![("Watch".into(), "https://twitch.tv/".into())];
        let value = activity(&c, "mp:external/x", "", ButtonStyle::Cdp);
        assert_eq!(value["flags"], 1);
        assert_eq!(value["application_id"], "1");
        assert_eq!(value["assets"]["large_image"], "mp:external/x");
        assert_eq!(value["buttons"][0], "Watch");
        assert_eq!(value["metadata"]["button_urls"][0], "https://twitch.tv/");
        assert!(value.get("timestamps").is_some());
    }

    #[test]
    fn the_pipe_gets_button_objects_not_labels() {
        let mut c = card();
        c.buttons = vec![("Watch".into(), "https://twitch.tv/".into())];
        let value = activity(&c, "https://example.test/logo.png", "", ButtonStyle::Rpc);
        // The pipe rejects a string array with 4000 and takes the whole card down.
        assert_eq!(value["buttons"][0]["label"], "Watch");
        assert_eq!(value["buttons"][0]["url"], "https://twitch.tv/");
        assert!(value.get("metadata").is_none());
    }

    #[test]
    fn listening_cards_get_an_end_for_the_progress_bar_and_spotify_flags() {
        let mut c = card();
        c.kind = 2;
        c.app_id = SPOTIFY_APP_ID;
        c.name = "Spotify".into();
        let value = activity(&c, "mp:external/x", "mp:external/y", ButtonStyle::Rpc);
        assert_eq!(value["type"], 2);
        assert_eq!(value["flags"], 48);
        assert!(value["timestamps"]["start"].as_u64().is_some());
        assert!(
            value["timestamps"]["end"].as_u64().unwrap()
                > value["timestamps"]["start"].as_u64().unwrap()
        );
    }

    #[test]
    fn spotify_carries_the_fields_its_integration_reads() {
        let mut c = card();
        c.kind = 2;
        c.app_id = SPOTIFY_APP_ID;
        let value = activity(&c, "mp:external/x", "", ButtonStyle::Cdp);
        assert_eq!(value["flags"], 48);
        assert!(value["sync_id"].as_str().is_some());
        assert_eq!(value["party"]["id"], "spotify:spider");
        assert_eq!(value["metadata"]["type"], "track");
        assert!(value["metadata"]["button_urls"].is_array());
    }

    #[test]
    fn streaming_carries_a_url_and_no_timer_is_omitted() {
        let mut c = card();
        c.kind = 1;
        c.stream_url = "https://www.twitch.tv/".into();
        c.timer = false;
        let value = activity(&c, "mp:external/x", "", ButtonStyle::Cdp);
        assert_eq!(value["type"], 1);
        assert_eq!(value["url"], "https://www.twitch.tv/");
        assert!(value.get("timestamps").is_none());
    }

    #[test]
    fn texts_keep_to_discords_limits() {
        let mut c = card();
        c.details = "x".into();
        c.state = "y".repeat(300);
        c.buttons = vec![
            ("A label much longer than thirty-two characters".into(), "https://a.test/".into()),
            ("Too long".into(), format!("https://b.test/{}", "z".repeat(600))),
            ("Third".into(), "https://c.test/".into()),
        ];
        let value = activity(&c, "", "", ButtonStyle::Rpc);
        // One character is padded to two (the pipe refuses one), 300 are cut to 128.
        assert_eq!(value["details"].as_str().unwrap().chars().count(), 2);
        assert_eq!(value["state"].as_str().unwrap().chars().count(), 128);
        // The label is cut, the over-long link is left out, two buttons at most.
        let buttons = value["buttons"].as_array().unwrap();
        assert_eq!(buttons.len(), 2);
        assert_eq!(buttons[0]["label"].as_str().unwrap().chars().count(), 32);
        assert_eq!(buttons[1]["label"], "Third");
    }

    #[test]
    fn clients_are_read_from_their_pages_and_pipes() {
        assert_eq!(main_window_host("https://discord.com/channels/@me"), Some("discord.com"));
        assert_eq!(main_window_host("https://ptb.discord.com/app"), Some("ptb.discord.com"));
        assert_eq!(main_window_host("https://discord.com/overlay"), None);
        assert_eq!(main_window_host("https://example.com/discord.com"), None);
        assert_eq!(Client::of_host("canary.discord.com"), Client::Canary);
        let ready = json!({ "evt": "READY", "data": { "config": { "api_endpoint": "//ptb.discord.com/api" },
            "user": { "username": "someone", "global_name": "Some One" } } });
        assert_eq!(Client::of_ready(&ready), Client::Ptb);
        assert_eq!(user_name(&ready), "Some One");
        for client in Client::ALL {
            assert_eq!(Client::parse(client.word()), client);
        }
    }
}
