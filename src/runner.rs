//! The presence loop: keeps `spider.json`'s cards on the profile.
//!
//! The file is the control surface: it is reloaded as soon as it changes, and the cards follow.
//! [`Runner::step`] does one turn and says how long to wait before the next, so the same loop
//! runs headless (`spider run`) and behind Spider's window ([`Worker`], on its own thread: the
//! transports block for a few seconds at most, the window must not).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::config::{self, Settings};
use crate::engine::{self, Client, Transport};

/// Set while cards are on the profile: closing the console clears them then.
pub static LIVE: AtomicBool = AtomicBool::new(false);
/// Set when the process is closing: no push may start after the last clear.
pub static CLOSING: AtomicBool = AtomicBool::new(false);

/// Where the loop stands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    /// No usable file yet.
    Waiting,
    /// `enabled` is false in the file.
    Disabled,
    /// Paused from the window.
    Paused,
    /// The first push is under way.
    Connecting,
    /// The cards are on the profile.
    Live,
    /// Discord refused the cards or could not be reached; tried again soon.
    Failing,
}

/// One card, as the window shows it.
#[derive(Clone, Debug)]
pub struct Card {
    pub slot: u8,
    pub kind: u8,
    pub name: String,
    pub details: String,
    pub state: String,
    pub app_id: u64,
    pub image: bool,
    pub small_image: bool,
    pub platform: Option<&'static str>,
    pub buttons: Vec<String>,
    pub timer: bool,
}

/// Everything the window shows about the loop.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub phase: Phase,
    pub transport: Option<Transport>,
    pub client: Client,
    pub account: Option<engine::Account>,
    pub cards: Vec<Card>,
    pub live_since: Option<Instant>,
    pub last_push: Option<Instant>,
    pub next_push: Option<Instant>,
    pub error: Option<String>,
    pub pushes: u64,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            phase: Phase::Waiting,
            transport: None,
            client: Client::Auto,
            account: None,
            cards: Vec::new(),
            live_since: None,
            last_push: None,
            next_push: None,
            error: None,
            pushes: 0,
        }
    }
}

/// The loop's state between two steps.
pub struct Runner {
    path: PathBuf,
    settings: Option<Settings>,
    stamp: Stamp,
    signature: String,
    /// The transport setting of the last push: a change is a hand-over.
    mode: Option<engine::Mode>,
    transport: Option<Transport>,
    applied: bool,
    failed: bool,
    paused: bool,
    force: bool,
    pushed_at: Option<Instant>,
    live_since: Option<Instant>,
    error: Option<String>,
    pushes: u64,
    /// Failed pushes in a row: the retry waits longer after each one.
    failures: u32,
}

impl Runner {
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            settings: None,
            stamp: Stamp::none(),
            signature: String::new(),
            mode: None,
            transport: None,
            applied: false,
            failed: false,
            paused: false,
            force: false,
            pushed_at: None,
            live_since: None,
            error: None,
            pushes: 0,
            failures: 0,
        }
    }

    /// How long until the next push: the refresh while live, the retry after a failure,
    /// doubled for each failure in a row up to a minute. Discord ignores an application's
    /// connections for about a minute after a few quick ones, and steady retries every five
    /// seconds would keep it silent.
    fn wait(&self, settings: &Settings) -> Duration {
        if self.failed || !self.applied {
            let base = settings.retry();
            let doubled = base * 2u32.pow(self.failures.saturating_sub(1).min(4));
            doubled.min(Duration::from_secs(60)).max(base)
        } else {
            settings.refresh()
        }
    }

    /// One turn of the loop; returns how long to wait before the next one.
    pub fn step(&mut self, log: &mut dyn FnMut(&str)) -> Duration {
        self.reload(log);
        let Some(current) = self.settings.as_ref() else {
            return Duration::from_millis(500);
        };
        if !current.enabled || self.paused {
            if self.applied || self.transport.is_some() {
                let why = if self.paused { "pause" } else { "désactivé" };
                match engine::clear_all() {
                    Ok(()) => log(&format!("{why} : cartes retirées")),
                    Err(error) => log(&format!("{why} : {error}")),
                }
                self.forget();
            }
            self.pushed_at = None;
            return Duration::from_millis(500);
        }
        let cards = match current.activities() {
            Ok(cards) => cards,
            Err(error) => {
                log(&format!("config : {error}"));
                self.settings = None;
                return Duration::from_millis(500);
            }
        };
        let mode = current.mode();
        let wait = self.wait(current);
        let next = cards
            .iter()
            .map(engine::Config::signature)
            .collect::<Vec<_>>()
            .join(";");
        // A save that only touched the window's theme or the refresh changes nothing here:
        // only the cards and the transport make a new push (and a hand-over).
        let changed = next != self.signature || self.mode != Some(mode);
        let due = self.pushed_at.is_none_or(|at| at.elapsed() >= wait);
        if (changed || due || self.force) && !CLOSING.load(Ordering::SeqCst) {
            self.force = false;
            self.signature = next;
            self.mode = Some(mode);
            self.pushed_at = Some(Instant::now());
            // The transport that worked stays first; a change of the cards while others are on
            // the profile clears them first, so the old type (Playing next to Streaming) cannot
            // stay behind. A plain refresh just sends the same cards again.
            let previous = if self.applied { self.transport } else { None };
            let handover = self.applied && changed;
            match engine::publish(&cards, mode, previous, handover) {
                Ok(used) => {
                    if !self.applied || self.transport != Some(used) {
                        self.live_since = Some(Instant::now());
                    }
                    self.applied = true;
                    self.failed = false;
                    self.failures = 0;
                    self.transport = Some(used);
                    self.error = None;
                    self.pushes += 1;
                    LIVE.store(true, Ordering::SeqCst);
                    let plural = if cards.len() > 1 { "s" } else { "" };
                    log(&format!(
                        "{} : {} carte{plural} en ligne",
                        used.label(),
                        cards.len()
                    ));
                }
                Err(error) => {
                    // `publish` already removed what landed: nothing is on the profile.
                    self.forget();
                    self.failed = true;
                    self.failures += 1;
                    log(&format!("échec : {error}"));
                    self.error = Some(error);
                }
            }
        }
        Duration::from_millis(250)
    }

    /// Reads the file again when it changed; a broken save keeps the last good cards.
    fn reload(&mut self, log: &mut dyn FnMut(&str)) {
        let now = Stamp::of(&self.path);
        if now == self.stamp {
            return;
        }
        self.stamp = now;
        if self.stamp.0.is_none() {
            if self.settings.is_none() {
                log(&format!(
                    "{} introuvable : configure Spider d'abord",
                    self.path.display()
                ));
            }
            return;
        }
        match Settings::load(&self.path) {
            Ok(next) => {
                let client = next.client();
                // The pipes belong to one client: switching clears the old account's cards.
                if client != engine::client() {
                    if self.applied {
                        let _ = engine::clear_all();
                        self.forget();
                    }
                    engine::set_client(client);
                }
                let plural = if next.activities.len() > 1 { "s" } else { "" };
                log(&format!(
                    "config : {} carte{plural}, transport {}, client {}",
                    next.activities.len(),
                    next.transport,
                    client.label()
                ));
                self.settings = Some(next);
                self.error = None;
            }
            Err(error) => log(&format!("config : {error}")),
        }
    }

    fn forget(&mut self) {
        self.applied = false;
        self.failed = false;
        self.transport = None;
        self.live_since = None;
        LIVE.store(false, Ordering::SeqCst);
    }

    /// Takes the cards off the profile until `set_paused(false)`.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        if !paused {
            self.force = true;
            self.failures = 0;
        }
    }

    /// Sends the cards again at the next step, without waiting for the refresh.
    pub fn republish(&mut self) {
        self.force = true;
    }

    /// Takes the cards off the profile for good (the loop is ending).
    pub fn stop(&mut self, log: &mut dyn FnMut(&str)) {
        CLOSING.store(true, Ordering::SeqCst);
        if self.applied || self.transport.is_some() || LIVE.load(Ordering::SeqCst) {
            match engine::clear_all() {
                Ok(()) => log("arrêt : cartes retirées"),
                Err(error) => log(&format!("arrêt : {error}")),
            }
        }
        self.forget();
        CLOSING.store(false, Ordering::SeqCst);
    }

    pub fn snapshot(&self) -> Snapshot {
        let cards = self
            .settings
            .as_ref()
            .map(|settings| {
                settings
                    .activities
                    .iter()
                    .enumerate()
                    .map(|(index, activity)| Card {
                        slot: if activity.slot == 0 {
                            index as u8 + 1
                        } else {
                            activity.slot
                        },
                        kind: config::kind(&activity.status),
                        name: activity.name.clone(),
                        details: activity.details.clone(),
                        state: activity.state.clone(),
                        app_id: activity.app_id,
                        image: !activity.image.trim().is_empty(),
                        small_image: !activity.small_image.trim().is_empty()
                            || config::platform(&activity.platform).is_some(),
                        platform: config::platform(&activity.platform),
                        buttons: activity
                            .buttons
                            .iter()
                            .filter(|button| {
                                !button.label.trim().is_empty() && config::is_url(&button.url)
                            })
                            .map(|button| button.label.clone())
                            .take(2)
                            .collect(),
                        timer: activity.timer,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let phase = match &self.settings {
            None => Phase::Waiting,
            Some(settings) if !settings.enabled => Phase::Disabled,
            Some(_) if self.paused => Phase::Paused,
            Some(_) if self.applied => Phase::Live,
            Some(_) if self.failed => Phase::Failing,
            Some(_) => Phase::Connecting,
        };
        let next_push = self
            .settings
            .as_ref()
            .and_then(|settings| self.pushed_at.map(|at| at + self.wait(settings)));
        Snapshot {
            phase,
            transport: self.transport,
            client: engine::client(),
            account: engine::account(),
            cards,
            live_since: self.live_since,
            last_push: self.pushed_at,
            next_push,
            error: self.error.clone(),
            pushes: self.pushes,
        }
    }
}

/// `(length, modified)` of the config file: enough to notice a save without hashing it.
#[derive(Clone, PartialEq, Eq)]
struct Stamp(Option<(u64, Option<std::time::SystemTime>)>);

impl Stamp {
    fn none() -> Self {
        Self(None)
    }

    fn of(path: &Path) -> Self {
        match std::fs::metadata(path) {
            Ok(meta) => Self(Some((meta.len(), meta.modified().ok()))),
            Err(_) => Self(None),
        }
    }
}

/// Clears the cards when the console window is closed (or Ctrl+C in `spider run`).
///
/// A debug-port card outlives the process, so without this closing the window left it on the
/// profile. Windows gives the handler a few seconds, plenty for the clear.
pub fn clear_on_close() {
    #[cfg(windows)]
    {
        unsafe extern "system" fn on_close(_event: u32) -> i32 {
            CLOSING.store(true, Ordering::SeqCst);
            if LIVE.swap(false, Ordering::SeqCst) {
                let _ = engine::clear_all();
            }
            // FALSE: the default handler then ends the process as usual.
            0
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn SetConsoleCtrlHandler(
                handler: Option<unsafe extern "system" fn(u32) -> i32>,
                add: i32,
            ) -> i32;
        }
        // SAFETY: the handler is a plain function that lives as long as the program.
        unsafe {
            SetConsoleCtrlHandler(Some(on_close), 1);
        }
    }
}

/// The local time as `HH:MM:SS`.
pub fn clock() -> String {
    #[cfg(windows)]
    {
        #[repr(C)]
        struct SystemTime {
            year: u16,
            month: u16,
            weekday: u16,
            day: u16,
            hour: u16,
            minute: u16,
            second: u16,
            millis: u16,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetLocalTime(time: *mut SystemTime);
        }
        let mut now = SystemTime {
            year: 0,
            month: 0,
            weekday: 0,
            day: 0,
            hour: 0,
            minute: 0,
            second: 0,
            millis: 0,
        };
        // SAFETY: GetLocalTime only fills the structure it is given.
        unsafe { GetLocalTime(&mut now) };
        format!("{:02}:{:02}:{:02}", now.hour, now.minute, now.second)
    }
    #[cfg(not(windows))]
    {
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        format!(
            "{:02}:{:02}:{:02}",
            seconds / 3600 % 24,
            seconds / 60 % 60,
            seconds % 60
        )
    }
}

/// The log beside the config: the local time, no repeated line, and the last lines kept for
/// the window.
pub struct Log {
    path: PathBuf,
    last: Option<String>,
    echo: bool,
    recent: VecDeque<String>,
}

impl Log {
    /// `echo` also prints each line (the headless `run`).
    pub fn new(config: &Path, echo: bool) -> Self {
        Self {
            path: config.with_extension("log"),
            last: None,
            echo,
            recent: VecDeque::new(),
        }
    }

    pub fn line(&mut self, text: &str) {
        if self.last.as_deref() == Some(text) {
            return;
        }
        self.last = Some(text.to_owned());
        let line = format!("[{}] {text}", clock());
        if self.echo {
            println!("{line}");
        }
        if self.recent.len() == 200 {
            self.recent.pop_front();
        }
        self.recent.push_back(line.clone());
        use std::io::Write as _;
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut file| {
                if file.metadata().map_or(0, |meta| meta.len()) > 512 * 1024 {
                    return Ok(());
                }
                writeln!(file, "{line}")
            });
    }

    pub fn recent(&self) -> &VecDeque<String> {
        &self.recent
    }
}

/// What the window asks the loop.
pub enum Command {
    Pause(bool),
    Republish,
    Stop,
}

/// What the loop shows the window.
#[derive(Default)]
pub struct Shared {
    pub snapshot: Snapshot,
    pub log: Vec<String>,
    /// Bumped on every change, so the window redraws only when something moved.
    pub version: u64,
}

/// The loop on its own thread, for the window.
pub struct Worker {
    commands: mpsc::Sender<Command>,
    shared: Arc<Mutex<Shared>>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(path: &Path) -> Self {
        let (commands, inbox) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let state = Arc::clone(&shared);
        let path = path.to_path_buf();
        let thread = std::thread::Builder::new()
            .name("spider-presence".to_owned())
            .spawn(move || {
                let mut runner = Runner::new(&path);
                let mut log = Log::new(&path, false);
                log.line("présence lancée");
                Self::share(&state, &runner, &log);
                loop {
                    // Each line shows at once: a push can take a few seconds when Discord is
                    // slow, and the window should say what is going on meanwhile.
                    let wait = runner.step(&mut |line| {
                        log.line(line);
                        Self::share_log(&state, &log);
                    });
                    Self::share(&state, &runner, &log);
                    match inbox.recv_timeout(wait) {
                        Ok(Command::Pause(paused)) => {
                            runner.set_paused(paused);
                            if !paused {
                                log.line("reprise");
                            }
                        }
                        Ok(Command::Republish) => runner.republish(),
                        Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => {
                            runner.stop(&mut |line| log.line(line));
                            Self::share(&state, &runner, &log);
                            break;
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                }
            })
            .expect("the presence thread starts");
        Self {
            commands,
            shared,
            thread: Some(thread),
        }
    }

    fn share(state: &Arc<Mutex<Shared>>, runner: &Runner, log: &Log) {
        if let Ok(mut shared) = state.lock() {
            shared.snapshot = runner.snapshot();
            shared.log = log.recent().iter().cloned().collect();
            shared.version = shared.version.wrapping_add(1);
        }
    }

    fn share_log(state: &Arc<Mutex<Shared>>, log: &Log) {
        if let Ok(mut shared) = state.lock() {
            shared.log = log.recent().iter().cloned().collect();
            shared.version = shared.version.wrapping_add(1);
        }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// The last snapshot, the log and the change counter.
    pub fn view(&self) -> (Snapshot, Vec<String>, u64) {
        match self.shared.lock() {
            Ok(shared) => (shared.snapshot.clone(), shared.log.clone(), shared.version),
            Err(_) => (Snapshot::default(), Vec::new(), 0),
        }
    }

    /// Takes the cards off the profile and waits for the thread.
    pub fn stop(mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
