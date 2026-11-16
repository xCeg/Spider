//! Spider's window: the opening, the menu, and every screen behind it.
//!
//! Each screen is a small loop: paint a canvas, show it (only the rows that changed), wait
//! for a key (or for the next frame while something moves). The presence itself runs on its
//! own thread ([`Worker`]), so the window stays smooth while Discord takes its time.

use std::io;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::logo;
use super::paint::{self, Canvas, Rgb, Style, THEMES, Theme, clip, ink, width};
use super::preview::{self, CardView};
use super::term::{Key, Term};
use crate::config::Settings;
use crate::doctor::{self, Level};
use crate::engine::{self, Client, PipeInfo};
use crate::runner::{self, Command, Phase, Snapshot, Worker};

/// One frame while something moves (about 60 a second).
pub(super) const FRAME: Duration = Duration::from_millis(16);
/// The wait between two frames while nothing moves (clocks still tick).
pub(super) const IDLE: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Tone {
    Good,
    Warn,
    Bad,
    Info,
}

impl Tone {
    pub(super) fn colour(self) -> Rgb {
        match self {
            Tone::Good => ink::GOOD,
            Tone::Warn => ink::WARN,
            Tone::Bad => ink::BAD,
            Tone::Info => ink::MUTED,
        }
    }
}

/// A short message under the menu, faded out after a few seconds.
pub(super) struct Notice {
    text: String,
    tone: Tone,
    at: Instant,
}

/// What the background probe saw of Discord.
#[derive(Clone, Default)]
pub(super) struct Probe {
    pub pipes: Vec<u8>,
    pub accounts: Vec<PipeInfo>,
    pub debug: Option<Client>,
    pub done: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Item {
    Start,
    Dashboard,
    Configure,
    Stop,
    Clear,
    Doctor,
    Theme,
    Restart,
    Quit,
}

pub(super) struct App {
    pub term: Term,
    pub path: PathBuf,
    pub settings: Option<Settings>,
    pub theme: &'static Theme,
    pub worker: Option<Worker>,
    pub probe: Arc<Mutex<Probe>>,
    born: Instant,
    /// When the menu first showed, for its entrance.
    menu_since: Option<Instant>,
    notice: Option<Notice>,
    pub selected: usize,
}

/// Opens the window and runs it until the user quits.
pub fn run(path: PathBuf) -> io::Result<()> {
    let term = Term::open("Spider")?;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        super::term::restore();
        if runner::LIVE.swap(false, Ordering::SeqCst) {
            let _ = engine::clear_all();
        }
        previous(info);
    }));
    let settings = Settings::load(&path).ok();
    let theme = settings
        .as_ref()
        .map_or(&THEMES[0], |settings| paint::theme(&settings.theme));
    if let Some(settings) = &settings {
        engine::set_client(settings.client());
    }
    let probe = start_probe();
    if let Some(settings) = &settings {
        warm_names(settings);
    }
    let mut app = App {
        term,
        path,
        settings,
        theme,
        worker: None,
        probe,
        born: Instant::now(),
        menu_since: None,
        notice: None,
        selected: 0,
    };
    let result = app.main();
    let farewell = app.outro();
    app.term.close();
    result.and(farewell)
}

/// Watches Discord every two seconds: which pipes exist and which client serves each (read
/// from the pipe's server process), and who has the debug port. It never shakes hands: Discord
/// answers only a couple of quick connections per application, and those belong to the cards.
fn start_probe() -> Arc<Mutex<Probe>> {
    let probe = Arc::new(Mutex::new(Probe::default()));
    let shared = Arc::clone(&probe);
    let _ = std::thread::Builder::new()
        .name("spider-probe".to_owned())
        .spawn(move || {
            let mut seen: Option<(Vec<u8>, Instant)> = None;
            loop {
                let pipes = engine::present_pipes();
                let debug = engine::debug_port_client();
                // The owners only change when the pipes do; the accounts fill in as the
                // presence connects, so they are read again now and then.
                let stale = seen.as_ref().is_none_or(|(known, at)| {
                    *known != pipes || at.elapsed() >= Duration::from_secs(6)
                });
                let accounts = stale.then(|| {
                    seen = Some((pipes.clone(), Instant::now()));
                    engine::clients()
                });
                if let Ok(mut probe) = shared.lock() {
                    probe.pipes = pipes;
                    probe.debug = debug;
                    if let Some(accounts) = accounts {
                        probe.accounts = accounts;
                    }
                    probe.done = true;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
    probe
}

impl App {
    fn main(&mut self) -> io::Result<()> {
        self.intro()?;
        if self.settings.is_none() {
            if self.path.exists() {
                if let Err(error) = Settings::load(&self.path) {
                    self.say(Tone::Bad, format!("Fichier illisible : {error}"));
                }
            } else if self.welcome()? {
                self.configure()?;
            }
        }
        if self.menu_since.is_none() {
            self.menu_since = Some(Instant::now());
        }
        loop {
            let item = self.menu()?;
            match item {
                Item::Quit => return Ok(()),
                Item::Start => self.start()?,
                Item::Dashboard => self.dashboard()?,
                Item::Configure => self.configure()?,
                Item::Stop => self.stop_presence()?,
                Item::Clear => {
                    match self.busy("Retrait", "Retrait des cartes de ton profil…", engine::clear_all)? {
                        Ok(()) => self.say(Tone::Good, "Cartes retirées de ton profil"),
                        Err(error) => self.say(Tone::Bad, format!("Retrait impossible : {error}")),
                    }
                }
                Item::Doctor => self.doctor()?,
                Item::Theme => self.themes()?,
                Item::Restart => self.restart_discord()?,
            }
        }
    }

    /// Whether the big logo fits above the menu.
    fn big(&self, canvas: &Canvas) -> bool {
        canvas.w >= logo::WIDTH + 8 && canvas.h >= 24
    }

    /// The row the big logo starts on, the same for the opening and the menu (no jump).
    fn hero_top(&self, canvas: &Canvas, below: i32) -> i32 {
        ((canvas.h - 1 - logo::HERO_HEIGHT - below) / 2).max(0)
    }

    /// The opening: the logo scans in, then the menu takes over without a jump.
    fn intro(&mut self) -> io::Result<()> {
        let start = Instant::now();
        loop {
            let t = start.elapsed().as_secs_f32();
            let mut canvas = self.term.canvas();
            let below = 2 + 1 + 2 + self.items().len() as i32 + 2;
            let top = if self.big(&canvas) {
                self.hero_top(&canvas, below)
            } else {
                ((canvas.h - logo::HERO_HEIGHT) / 2).max(0)
            };
            logo::hero(&mut canvas, top, self.theme, t, logo::opening_shine(t));
            self.term.present(&canvas, None)?;
            if t >= logo::INTRO {
                return Ok(());
            }
            if let Some(key) = self.term.key(FRAME)?
                && key != Key::Resize
            {
                return Ok(());
            }
        }
    }

    /// The farewell: the logo leaves the way it came while the cards are taken down.
    fn outro(&mut self) -> io::Result<()> {
        let worker = self.worker.take();
        let job = std::thread::spawn(move || {
            if let Some(worker) = worker {
                worker.stop();
            }
        });
        let start = Instant::now();
        loop {
            let k = start.elapsed().as_secs_f32() / 0.6;
            let mut canvas = self.term.canvas();
            if self.big(&canvas) {
                let below = 2 + 1 + 2 + 7 + 2;
                let top = self.hero_top(&canvas, below);
                logo::hero(&mut canvas, top, self.theme, logo::INTRO * (1.0 - k).max(0.0), None);
            }
            if k >= 1.0 {
                if job.is_finished() {
                    break;
                }
                canvas.centre(canvas.h / 2, "Retrait des cartes…", Style::fg(ink::MUTED));
            }
            self.term.present(&canvas, None)?;
            let _ = self.term.key(FRAME)?;
        }
        let _ = job.join();
        Ok(())
    }

    /// The first start: what Spider does, before the first questions.
    fn welcome(&mut self) -> io::Result<bool> {
        let since = Instant::now();
        let lines: [(&str, Rgb, bool); 6] = [
            ("Bienvenue sur Spider.", ink::TEXT, true),
            ("", ink::TEXT, false),
            ("Spider affiche une carte d'activité sur ton profil Discord, sans token.", ink::MUTED, false),
            ("On la prépare ensemble en quelques questions : il te faut l'ID d'une", ink::MUTED, false),
            ("application Discord, ou l'un des modèles tout prêts (Spotify, YouTube…).", ink::MUTED, false),
            ("", ink::TEXT, false),
        ];
        loop {
            let seconds = self.born.elapsed().as_secs_f32();
            let mut canvas = self.term.canvas();
            let below = 2 + lines.len() as i32;
            let mut y;
            if self.big(&canvas) {
                let top = self.hero_top(&canvas, below);
                logo::hero(&mut canvas, top, self.theme, logo::INTRO, logo::idle_shine(seconds));
                y = top + logo::HERO_HEIGHT + 2;
            } else {
                logo::header(&mut canvas, self.theme, "Bienvenue");
                y = 4;
            }
            let shown = since.elapsed().as_secs_f32();
            for (index, (text, colour, bold)) in lines.iter().enumerate() {
                let k = ((shown - index as f32 * 0.07) / 0.3).clamp(0.0, 1.0);
                let mut style = Style::fg(colour.shade(1.0 - k));
                style.bold = *bold;
                canvas.centre(y, text, style);
                y += 1;
            }
            logo::footer(&mut canvas, &[("Entrée", "commencer"), ("Échap", "plus tard")]);
            self.term.present(&canvas, None)?;
            let moving = shown < 0.8 || logo::idle_shine(seconds).is_some();
            match self.term.key(if moving { FRAME } else { IDLE })? {
                Some(Key::Enter | Key::Char(' ')) => return Ok(true),
                Some(Key::Esc | Key::Ctrl('c')) => return Ok(false),
                _ => {}
            }
        }
    }

    // --- The menu -----------------------------------------------------------

    fn items(&self) -> Vec<(Item, &'static str, String)> {
        let mut list = Vec::new();
        let live = self.worker.as_ref().map(|worker| worker.view().0);
        match &live {
            Some(snapshot) => list.push((Item::Dashboard, "Tableau de bord", live_summary(snapshot))),
            None => list.push((
                Item::Start,
                "Lancer la présence",
                "publie tes cartes et les garde à jour".to_owned(),
            )),
        }
        list.push((
            Item::Configure,
            "Configurer",
            if self.settings.is_some() {
                "modifier les cartes et les réglages"
            } else {
                "créer ta première carte"
            }
            .to_owned(),
        ));
        if live.is_some() {
            list.push((Item::Stop, "Arrêter la présence", "retire les cartes de ton profil".to_owned()));
        } else {
            list.push((
                Item::Clear,
                "Retirer les cartes",
                "nettoie ton profil (pipe et port de debug)".to_owned(),
            ));
        }
        list.push((
            Item::Doctor,
            "Diagnostic",
            "Discord, comptes, port de debug, test réel".to_owned(),
        ));
        list.push((Item::Theme, "Thème", format!("{} · change les couleurs", self.theme.name)));
        list.push((
            Item::Restart,
            "Relancer Discord",
            "avec le port de debug : Streaming, badges, 3 cartes".to_owned(),
        ));
        list.push((
            Item::Quit,
            "Quitter",
            if live.is_some() {
                "retire les cartes et ferme Spider"
            } else {
                "à la prochaine"
            }
            .to_owned(),
        ));
        list
    }

    fn menu(&mut self) -> io::Result<Item> {
        loop {
            let items = self.items();
            self.selected = self.selected.min(items.len() - 1);
            let seconds = self.born.elapsed().as_secs_f32();
            let mut canvas = self.term.canvas();
            self.draw_menu(&mut canvas, &items, seconds);
            self.term.present(&canvas, None)?;
            let entering = self
                .menu_since
                .is_some_and(|since| since.elapsed().as_secs_f32() < 0.7);
            let moving = entering || logo::idle_shine(seconds).is_some() || self.notice_fading();
            let Some(key) = self.term.key(if moving { FRAME } else { IDLE })? else {
                continue;
            };
            let count = items.len();
            match key {
                Key::Up | Key::BackTab => self.selected = (self.selected + count - 1) % count,
                Key::Down | Key::Tab => self.selected = (self.selected + 1) % count,
                Key::Home | Key::PageUp => self.selected = 0,
                Key::End | Key::PageDown => self.selected = count - 1,
                Key::Enter | Key::Char(' ') => return Ok(items[self.selected].0),
                Key::Char(digit @ '1'..='9') => {
                    let index = digit as usize - '1' as usize;
                    if index < count {
                        self.selected = index;
                        return Ok(items[index].0);
                    }
                }
                Key::Esc | Key::Char('q' | 'Q') | Key::Ctrl('c') => return Ok(Item::Quit),
                _ => {}
            }
        }
    }

    fn draw_menu(&self, canvas: &mut Canvas, items: &[(Item, &'static str, String)], seconds: f32) {
        let below = 2 + 1 + 2 + items.len() as i32 + 2;
        let mut y = if self.big(canvas) {
            let top = self.hero_top(canvas, below);
            logo::hero(canvas, top, self.theme, logo::INTRO, logo::idle_shine(seconds));
            top + logo::HERO_HEIGHT + 2
        } else {
            logo::header(canvas, self.theme, "Menu");
            4
        };
        self.draw_status(canvas, y);
        y += 3;
        let entered = self
            .menu_since
            .map_or(1.0, |since| since.elapsed().as_secs_f32());
        let block = 72.min(canvas.w - 6);
        let x = (canvas.w - block) / 2;
        let column = 25;
        for (index, (_, label, detail)) in items.iter().enumerate() {
            let row = y + index as i32;
            // The rows come in one after the other.
            let k = ((entered - index as f32 * 0.05) / 0.25).clamp(0.0, 1.0);
            let fade = |colour: Rgb| colour.shade(1.0 - k);
            let selected = index == self.selected;
            if selected {
                canvas.fill(x - 1, row, block + 2, 1, fade(self.theme.accent().shade(0.8)));
                canvas.put(x - 1, row, '▌', Style::fg(fade(self.theme.accent())));
            }
            canvas.text(x + 1, row, &(index + 1).to_string(), Style::fg(fade(ink::FAINT)));
            let label_style = if selected {
                Style::fg(fade(ink::TEXT)).bold()
            } else {
                Style::fg(fade(ink::MUTED))
            };
            canvas.text(x + 4, row, label, label_style);
            let detail_colour = if selected { ink::MUTED } else { ink::FAINT };
            canvas.text(
                x + 4 + column,
                row,
                &clip(detail, block - column - 5),
                Style::fg(fade(detail_colour)),
            );
        }
        y += items.len() as i32 + 1;
        self.draw_notice(canvas, y);
        logo::footer(
            canvas,
            &[("↑↓", "choisir"), ("Entrée", "valider"), ("1-7", "raccourcis"), ("Échap", "quitter")],
        );
    }

    /// The state of things in one line: presence, Discord account, debug port, cards.
    fn draw_status(&self, canvas: &mut Canvas, y: i32) {
        let segments = self.status();
        let total: i32 = segments.iter().map(|(_, text)| 2 + width(text)).sum::<i32>()
            + 5 * (segments.len() as i32 - 1).max(0);
        let mut x = ((canvas.w - total) / 2).max(1);
        for (index, (colour, text)) in segments.iter().enumerate() {
            if index > 0 {
                x += 5;
            }
            canvas.put(x, y, '●', Style::fg(*colour));
            x = canvas.text(x + 2, y, text, Style::fg(ink::MUTED));
        }
    }

    fn status(&self) -> Vec<(Rgb, String)> {
        let probe = self.probe.lock().map(|probe| probe.clone()).unwrap_or_default();
        let mut list = Vec::new();
        if let Some(worker) = &self.worker {
            let (snapshot, _, _) = worker.view();
            list.push(match snapshot.phase {
                Phase::Live => (
                    ink::GOOD,
                    format!(
                        "En ligne · {}",
                        snapshot.transport.map_or("…", |transport| transport.label())
                    ),
                ),
                Phase::Failing => (ink::BAD, "Échec, nouvel essai…".to_owned()),
                Phase::Paused => (ink::WARN, "En pause".to_owned()),
                Phase::Disabled => (ink::FAINT, "Désactivée".to_owned()),
                Phase::Waiting | Phase::Connecting => (ink::WARN, "Connexion…".to_owned()),
            });
        }
        let chosen = self.settings.as_ref().map_or(Client::Auto, Settings::client);
        if !probe.done {
            list.push((ink::FAINT, "Discord…".to_owned()));
        } else if probe.pipes.is_empty() {
            list.push((ink::BAD, "Discord fermé".to_owned()));
        } else {
            list.push(discord_status(&probe, chosen));
        }
        list.push(match probe.debug {
            Some(_) => (ink::GOOD, "Port de debug ouvert".to_owned()),
            None => (ink::FAINT, "Port de debug fermé".to_owned()),
        });
        list.push(match &self.settings {
            Some(settings) => {
                let count = settings.activities.len();
                (ink::GOOD, format!("{count} carte{}", if count > 1 { "s" } else { "" }))
            }
            None => (ink::WARN, "Pas encore configuré".to_owned()),
        });
        list
    }

    /// Shows a short message under the menu.
    pub(super) fn say(&mut self, tone: Tone, text: impl Into<String>) {
        self.notice = Some(Notice {
            text: text.into(),
            tone,
            at: Instant::now(),
        });
    }

    fn notice_fading(&self) -> bool {
        self.notice
            .as_ref()
            .is_some_and(|notice| (5.0..6.5).contains(&notice.at.elapsed().as_secs_f32()))
    }

    fn draw_notice(&self, canvas: &mut Canvas, y: i32) {
        let Some(notice) = &self.notice else {
            return;
        };
        let age = notice.at.elapsed().as_secs_f32();
        let k = (1.0 - (age - 5.0) / 1.5).clamp(0.0, 1.0);
        if k <= 0.0 {
            return;
        }
        let message = clip(&notice.text, canvas.w - 10);
        let x = (canvas.w - 3 - width(&message)) / 2;
        canvas.put(x, y, '●', Style::fg(notice.tone.colour().shade(1.0 - k)));
        canvas.text(x + 3, y, &message, Style::fg(ink::TEXT.shade(1.0 - k)));
    }

    // --- Shared pieces ------------------------------------------------------

    /// Runs `task` on its own thread while a light runs along a line.
    pub(super) fn busy<T: Send + 'static>(
        &mut self,
        title: &str,
        detail: &str,
        task: impl FnOnce() -> T + Send + 'static,
    ) -> io::Result<T> {
        let job = std::thread::spawn(task);
        let start = Instant::now();
        while !job.is_finished() {
            let mut canvas = self.term.canvas();
            logo::header(&mut canvas, self.theme, title);
            let y = canvas.h / 2 - 1;
            comet(&mut canvas, y, self.theme, start.elapsed().as_secs_f32());
            canvas.centre(y + 2, detail, Style::fg(ink::MUTED));
            self.term.present(&canvas, None)?;
            let _ = self.term.key(FRAME)?;
        }
        match job.join() {
            Ok(value) => Ok(value),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    /// Asks a yes/no question; the safe answer is selected first.
    pub(super) fn confirm(&mut self, title: &str, lines: &[&str], yes: &str, no: &str) -> io::Result<bool> {
        let mut choice = 1;
        loop {
            let mut canvas = self.term.canvas();
            logo::header(&mut canvas, self.theme, title);
            let inner = lines.iter().map(|line| width(line)).max().unwrap_or(0).max(30);
            let w = (inner + 8).min(canvas.w - 4);
            let h = lines.len() as i32 + 6;
            let x = (canvas.w - w) / 2;
            let y = ((canvas.h - h) / 2).max(4);
            canvas.fill(x, y, w, h, ink::CARD_EDGE);
            canvas.frame(x, y, w, h, Style::fg(self.theme.accent().shade(0.45)));
            for (index, line) in lines.iter().enumerate() {
                let line = clip(line, w - 4);
                canvas.text(x + (w - width(&line)) / 2, y + 2 + index as i32, &line, Style::fg(ink::TEXT));
            }
            let buttons = [format!("  {yes}  "), format!("  {no}  ")];
            let total = width(&buttons[0]) + width(&buttons[1]) + 4;
            let mut bx = x + (w - total) / 2;
            let by = y + h - 2;
            for (index, label) in buttons.iter().enumerate() {
                let style = if index == choice {
                    Style::fg(ink::TEXT).bold().on(self.theme.accent().shade(0.55))
                } else {
                    Style::fg(ink::MUTED).on(ink::GHOST)
                };
                bx = canvas.text(bx, by, label, style) + 4;
            }
            logo::footer(&mut canvas, &[("←→", "choisir"), ("Entrée", "valider"), ("Échap", "annuler")]);
            self.term.present(&canvas, None)?;
            match self.term.key(IDLE)? {
                Some(Key::Left | Key::Right | Key::Tab | Key::BackTab) => choice = 1 - choice,
                Some(Key::Enter | Key::Char(' ')) => return Ok(choice == 0),
                Some(Key::Char('o' | 'O' | 'y' | 'Y')) => return Ok(true),
                Some(Key::Esc | Key::Char('n' | 'N') | Key::Ctrl('c')) => return Ok(false),
                _ => {}
            }
        }
    }

    // --- Presence -----------------------------------------------------------

    fn start(&mut self) -> io::Result<()> {
        if self.settings.is_none() {
            self.say(Tone::Warn, "Configure d'abord ta carte");
            return self.configure();
        }
        if self.worker.is_none() {
            self.worker = Some(Worker::start(&self.path));
        }
        self.dashboard()
    }

    fn stop_presence(&mut self) -> io::Result<()> {
        if let Some(worker) = self.worker.take() {
            self.busy("Arrêt", "Retrait des cartes de ton profil…", move || worker.stop())?;
            self.say(Tone::Good, "Présence arrêtée : cartes retirées");
            self.selected = 0;
        }
        Ok(())
    }

    fn dashboard(&mut self) -> io::Result<()> {
        loop {
            let Some((snapshot, log, _)) = self.worker.as_ref().map(Worker::view) else {
                return Ok(());
            };
            let mut canvas = self.term.canvas();
            self.draw_dashboard(&mut canvas, &snapshot, &log);
            self.term.present(&canvas, None)?;
            let Some(key) = self.term.key(Duration::from_millis(200))? else {
                continue;
            };
            let worker = self.worker.as_ref();
            match key {
                Key::Char('p' | 'P' | ' ') => {
                    if let Some(worker) = worker {
                        worker.send(Command::Pause(snapshot.phase != Phase::Paused));
                    }
                }
                Key::Char('r' | 'R') => {
                    if let Some(worker) = worker {
                        worker.send(Command::Republish);
                    }
                }
                Key::Char('e' | 'E') => self.configure()?,
                Key::Char('x' | 'X' | 's' | 'S') => {
                    self.stop_presence()?;
                    return Ok(());
                }
                Key::Esc | Key::Enter | Key::Char('q' | 'Q') | Key::Ctrl('c') => return Ok(()),
                _ => {}
            }
        }
    }

    fn draw_dashboard(&self, canvas: &mut Canvas, snapshot: &Snapshot, log: &[String]) {
        logo::header(canvas, self.theme, "Présence");
        let (colour, word) = match snapshot.phase {
            Phase::Live => (ink::GOOD, "EN LIGNE"),
            Phase::Failing => (ink::BAD, "ÉCHEC"),
            Phase::Paused => (ink::WARN, "EN PAUSE"),
            Phase::Disabled => (ink::FAINT, "DÉSACTIVÉE"),
            Phase::Waiting | Phase::Connecting => (ink::WARN, "CONNEXION…"),
        };
        // A slow pulse on the dot while live.
        let pulse = if snapshot.phase == Phase::Live {
            0.25 * (1.0 + (self.born.elapsed().as_secs_f32() * 2.4).sin())
        } else {
            0.0
        };
        canvas.put(4, 4, '●', Style::fg(colour.shade(pulse)));
        let after = canvas.text(6, 4, word, Style::fg(colour).bold());
        let mut details = Vec::new();
        if let Some(transport) = snapshot.transport {
            details.push(format!("par le {}", transport.label()));
        }
        if let Some(account) = &snapshot.account {
            details.push(format!("{} · {}", account.client.label(), account.name));
        }
        if let Some(since) = snapshot.live_since {
            details.push(format!("depuis {}", preview::clock(since.elapsed().as_secs())));
        }
        if let Some(next) = snapshot.next_push {
            let left = next.saturating_duration_since(Instant::now()).as_secs();
            details.push(format!("prochain envoi dans {left} s"));
        }
        canvas.text(after + 3, 4, &clip(&details.join("   ·   "), canvas.w - after - 6), Style::fg(ink::MUTED));
        let mut y = 6;
        if let Some(error) = &snapshot.error {
            canvas.text(4, y, &clip(error, canvas.w - 8), Style::fg(ink::BAD));
            y += 2;
        }
        // The cards side by side when they fit, one under the other otherwise.
        let views: Vec<CardView> = snapshot
            .cards
            .iter()
            .map(|card| CardView {
                kind: card.kind,
                title: card_title(&card.name, card.app_id),
                details: card.details.clone(),
                state: card.state.clone(),
                image: card.image,
                small_image: card.small_image,
                platform: card.platform,
                buttons: card.buttons.clone(),
                elapsed: card
                    .timer
                    .then(|| snapshot.live_since.map_or(0, |since| since.elapsed().as_secs())),
            })
            .collect();
        let across = ((canvas.w - 6) / (preview::WIDTH + 2)).max(1) as usize;
        let mut tallest = 0;
        for (index, view) in views.iter().enumerate() {
            let column = (index % across) as i32;
            if index > 0 && column == 0 {
                y += tallest + 1;
                tallest = 0;
            }
            let x = 4 + column * (preview::WIDTH + 2);
            canvas.text(x, y, &format!("Carte {}", index + 1), Style::fg(ink::FAINT));
            preview::draw(canvas, x, y + 1, view, self.theme);
            tallest = tallest.max(preview::height(view) + 1);
        }
        y += tallest + 1;
        // The last lines of the log.
        let room = canvas.h - 2 - y - 1;
        if room > 0 {
            canvas.text(4, y, "JOURNAL", Style::fg(ink::FAINT).bold());
            for (index, line) in log.iter().rev().take(room as usize).rev().enumerate() {
                canvas.text(4, y + 1 + index as i32, &clip(line, canvas.w - 8), Style::fg(ink::MUTED));
            }
        }
        let pause = if snapshot.phase == Phase::Paused { "reprendre" } else { "pause" };
        logo::footer(
            canvas,
            &[("P", pause), ("R", "renvoyer"), ("E", "modifier"), ("X", "arrêter"), ("Échap", "menu")],
        );
    }

    // --- Diagnostic -----------------------------------------------------------

    fn doctor(&mut self) -> io::Result<()> {
        let path = self.path.clone();
        let mut report = self.busy(
            "Diagnostic",
            "Vérification de Discord, des comptes et des applications…",
            move || doctor::check(&path),
        )?;
        loop {
            let mut canvas = self.term.canvas();
            logo::header(&mut canvas, self.theme, "Diagnostic");
            let mut y = 4;
            let presence = self.worker.as_ref().map(|worker| worker.view().0);
            for section in &report {
                if y >= canvas.h - 3 {
                    break;
                }
                canvas.text(4, y, &section.title.to_uppercase(), Style::fg(self.theme.accent()).bold());
                y += 1;
                for line in &section.lines {
                    if y >= canvas.h - 3 {
                        break;
                    }
                    let colour = match line.level {
                        Level::Good => ink::GOOD,
                        Level::Warn => ink::WARN,
                        Level::Bad => ink::BAD,
                        Level::Off => ink::FAINT,
                    };
                    canvas.put(6, y, '●', Style::fg(colour));
                    canvas.text(8, y, &clip(&line.label, 18), Style::fg(ink::TEXT));
                    canvas.text(28, y, &clip(&line.detail, canvas.w - 32), Style::fg(ink::MUTED));
                    y += 1;
                }
                y += 1;
            }
            if y < canvas.h - 3 {
                canvas.text(4, y, "PRÉSENCE", Style::fg(self.theme.accent()).bold());
                let (colour, text) = match &presence {
                    Some(snapshot) if snapshot.phase == Phase::Live => (ink::GOOD, live_summary(snapshot)),
                    Some(snapshot) => (ink::WARN, live_summary(snapshot)),
                    None => (ink::FAINT, "arrêtée : « T » envoie une vraie carte pendant 8 s".to_owned()),
                };
                canvas.put(6, y + 1, '●', Style::fg(colour));
                canvas.text(8, y + 1, &clip(&text, canvas.w - 12), Style::fg(ink::MUTED));
            }
            logo::footer(
                &mut canvas,
                &[("T", "tester pour de vrai"), ("R", "relancer le diagnostic"), ("Échap", "retour")],
            );
            self.term.present(&canvas, None)?;
            match self.term.key(IDLE)? {
                Some(Key::Char('t' | 'T')) => self.test_card()?,
                Some(Key::Char('r' | 'R')) => {
                    let path = self.path.clone();
                    report = self.busy(
                        "Diagnostic",
                        "Vérification de Discord, des comptes et des applications…",
                        move || doctor::check(&path),
                    )?;
                }
                Some(Key::Esc | Key::Enter | Key::Char('q' | 'Q') | Key::Ctrl('c')) => return Ok(()),
                _ => {}
            }
        }
    }

    /// Publishes the real cards for a few seconds, then takes them down: the proof that
    /// Discord accepts them, on the account it shows.
    fn test_card(&mut self) -> io::Result<()> {
        if self.worker.is_some() {
            self.say(Tone::Info, "La présence tourne déjà : regarde ton profil");
            return Ok(());
        }
        let settings = match Settings::load(&self.path) {
            Ok(settings) => settings,
            Err(error) => {
                self.say(Tone::Bad, format!("Pas de test : {error}"));
                return Ok(());
            }
        };
        let cards = match settings.activities() {
            Ok(cards) => cards,
            Err(error) => {
                self.say(Tone::Bad, format!("Pas de test : {error}"));
                return Ok(());
            }
        };
        let mode = settings.mode();
        engine::set_client(settings.client());
        let views: Vec<CardView> = settings.activities.iter().map(view_of).collect();
        let sent = self.busy("Test", "Envoi de la carte à Discord…", move || {
            engine::publish(&cards, mode, None, false)
        })?;
        let transport = match sent {
            Ok(transport) => transport,
            Err(error) => {
                self.say(Tone::Bad, format!("Discord a refusé : {error}"));
                return Ok(());
            }
        };
        runner::LIVE.store(true, Ordering::SeqCst);
        let account = engine::account();
        let start = Instant::now();
        const SHOWN: f32 = 8.0;
        loop {
            let elapsed = start.elapsed().as_secs_f32();
            if elapsed >= SHOWN {
                break;
            }
            let mut canvas = self.term.canvas();
            logo::header(&mut canvas, self.theme, "Test");
            canvas.put(4, 4, '●', Style::fg(ink::GOOD));
            canvas.text(6, 4, "CARTE ACCEPTÉE PAR DISCORD", Style::fg(ink::GOOD).bold());
            let whom = account
                .as_ref()
                .map(|account| format!(" sur {} · {}", account.client.label(), account.name))
                .unwrap_or_default();
            canvas.text(
                6,
                5,
                &format!("par le {}{whom} : regarde ton profil", transport.label()),
                Style::fg(ink::MUTED),
            );
            let left = (SHOWN - elapsed).ceil() as i32;
            canvas.text(6, 6, &format!("Retirée dans {left} s"), Style::fg(ink::FAINT));
            // A bar emptying with the time left.
            let bar = 40.min(canvas.w - 12);
            let filled = ((1.0 - elapsed / SHOWN) * bar as f32).round() as i32;
            for i in 0..bar {
                let colour = if i < filled { self.theme.at(i as f32 / bar as f32) } else { ink::GHOST };
                canvas.put(6 + i, 7, '─', Style::fg(colour));
            }
            for (index, view) in views.iter().enumerate() {
                let x = 6 + index as i32 * (preview::WIDTH + 2);
                if x + preview::WIDTH < canvas.w {
                    preview::draw(&mut canvas, x, 9, view, self.theme);
                }
            }
            logo::footer(&mut canvas, &[("Entrée", "retirer maintenant")]);
            self.term.present(&canvas, None)?;
            if let Some(Key::Enter | Key::Esc | Key::Char(' ')) = self.term.key(Duration::from_millis(100))? {
                break;
            }
        }
        let cleared = self.busy("Test", "Retrait de la carte de test…", engine::clear_all)?;
        runner::LIVE.store(false, Ordering::SeqCst);
        match cleared {
            Ok(()) => self.say(Tone::Good, "Test réussi : Discord a accepté la carte, elle est retirée"),
            Err(error) => self.say(Tone::Warn, format!("Carte acceptée, mais retrait incomplet : {error}")),
        }
        Ok(())
    }

    fn restart_discord(&mut self) -> io::Result<()> {
        let sure = self.confirm(
            "Relancer Discord",
            &[
                "Discord va se fermer, puis se rouvrir avec le port de debug.",
                "Un appel ou un vocal en cours sera coupé.",
                "",
                "Tant que ce port est ouvert, un programme de ce PC peut piloter",
                "Discord : ne le garde pas si tu n'en as pas besoin.",
            ],
            "Relancer",
            "Annuler",
        )?;
        if !sure {
            return Ok(());
        }
        if let Some(settings) = &self.settings
            && self.worker.is_none()
        {
            engine::set_client(settings.client());
        }
        match self.busy(
            "Relance",
            "Fermeture puis relance de Discord (jusqu'à 25 s)…",
            engine::restart_with_debug_port,
        )? {
            Ok(client) => self.say(Tone::Good, format!("{} relancé avec le port de debug", client.label())),
            Err(error) => self.say(Tone::Bad, format!("Relance impossible : {error}")),
        }
        Ok(())
    }

    // --- Themes ---------------------------------------------------------------

    fn themes(&mut self) -> io::Result<()> {
        let original = self.theme;
        let mut index = original.index();
        let since = Instant::now();
        loop {
            let theme = &THEMES[index];
            self.theme = theme;
            let seconds = since.elapsed().as_secs_f32();
            let mut canvas = self.term.canvas();
            logo::header(&mut canvas, theme, "Thème");
            let x = (canvas.w - logo::WIDTH) / 2;
            let band = Some(-8.0 + (seconds % 2.4) / 1.4 * (logo::WIDTH as f32 + 16.0));
            logo::draw(&mut canvas, x, 4, theme, logo::INTRO, band);
            let list_x = (canvas.w - 44) / 2;
            for (row, each) in THEMES.iter().enumerate() {
                let y = 12 + row as i32;
                let selected = row == index;
                if selected {
                    canvas.fill(list_x - 1, y, 46, 1, each.accent().shade(0.8));
                    canvas.put(list_x - 1, y, '▌', Style::fg(each.accent()));
                }
                let style = if selected { Style::fg(ink::TEXT).bold() } else { Style::fg(ink::MUTED) };
                canvas.text(list_x + 2, y, each.name, style);
                for i in 0..24 {
                    canvas.put(list_x + 16 + i, y, '█', Style::fg(each.at(i as f32 / 23.0)));
                }
                if each.word == original.word {
                    canvas.text(list_x + 42, y, "●", Style::fg(ink::FAINT));
                }
            }
            logo::footer(&mut canvas, &[("↑↓", "choisir"), ("Entrée", "garder"), ("Échap", "annuler")]);
            self.term.present(&canvas, None)?;
            let Some(key) = self.term.key(FRAME)? else {
                continue;
            };
            match key {
                Key::Up | Key::Left | Key::BackTab => index = (index + THEMES.len() - 1) % THEMES.len(),
                Key::Down | Key::Right | Key::Tab => index = (index + 1) % THEMES.len(),
                Key::Enter | Key::Char(' ') => {
                    if let Some(settings) = &mut self.settings {
                        settings.theme = theme.word.to_owned();
                        if let Err(error) = settings.save(&self.path) {
                            self.say(Tone::Bad, format!("Thème non enregistré : {error}"));
                            return Ok(());
                        }
                    }
                    self.say(Tone::Good, format!("Thème {} gardé", theme.name));
                    return Ok(());
                }
                Key::Esc | Key::Ctrl('c') => {
                    self.theme = original;
                    return Ok(());
                }
                _ => {}
            }
        }
    }
}

/// A line of light running back and forth along a rule: the "please wait".
fn comet(canvas: &mut Canvas, y: i32, theme: &Theme, t: f32) {
    let length = 32;
    let x = (canvas.w - length) / 2;
    let phase = (t * 0.9) % 2.0;
    let head = if phase < 1.0 { phase } else { 2.0 - phase };
    let head = head * (length - 1) as f32;
    for i in 0..length {
        let distance = (i as f32 - head).abs();
        let glow = (1.0 - distance / 8.0).max(0.0);
        let colour = theme.at(i as f32 / (length - 1) as f32).shade(0.82 - 0.82 * glow);
        canvas.put(x + i, y, '─', Style::fg(colour));
    }
}

/// Asks Discord for the cards' application names in the background, so the previews show
/// the real titles instead of a placeholder.
pub(super) fn warm_names(settings: &Settings) {
    let ids: Vec<u64> = settings
        .activities
        .iter()
        .map(|activity| activity.app_id)
        .filter(|id| *id != 0 && engine::known_application_name(*id).is_none())
        .collect();
    if !ids.is_empty() {
        std::thread::spawn(move || {
            for id in ids {
                let _ = engine::application_name(id);
            }
        });
    }
}

/// A card's title: its own name, else the application's (once known), else a placeholder.
pub(super) fn card_title(name: &str, app_id: u64) -> String {
    if !name.trim().is_empty() {
        return name.trim().to_owned();
    }
    engine::known_application_name(app_id).unwrap_or_else(|| "Ton application".to_owned())
}

/// The card a file's activity describes, clock at zero.
pub(super) fn view_of(activity: &crate::config::Activity) -> CardView {
    CardView {
        kind: crate::config::kind(&activity.status),
        title: card_title(&activity.name, activity.app_id),
        details: activity.details.clone(),
        state: activity.state.clone(),
        image: !activity.image.trim().is_empty(),
        small_image: !activity.small_image.trim().is_empty(),
        platform: crate::config::platform(&activity.platform),
        buttons: activity
            .buttons
            .iter()
            .filter(|button| !button.label.trim().is_empty())
            .map(|button| button.label.clone())
            .take(2)
            .collect(),
        elapsed: activity.timer.then_some(0),
    }
}

/// The Discord part of the status line: the account the cards go to when known, else the
/// clients that are open.
fn discord_status(probe: &Probe, chosen: Client) -> (Rgb, String) {
    let short = |client: Client| match client {
        Client::Ptb => "PTB",
        Client::Canary => "Canary",
        _ => "Discord",
    };
    if let Some(account) = engine::account()
        && (chosen == Client::Auto || account.client == chosen)
    {
        return (ink::GOOD, format!("{} · {}", account.client.label(), account.name));
    }
    match chosen {
        Client::Auto if probe.accounts.is_empty() => (ink::GOOD, "Discord ouvert".to_owned()),
        Client::Auto => {
            let names: Vec<&str> = probe.accounts.iter().map(|info| short(info.client)).collect();
            (ink::GOOD, format!("{} ouvert{}", names.join(" + "), if names.len() > 1 { "s" } else { "" }))
        }
        wanted => match probe.accounts.iter().find(|info| info.client == wanted) {
            Some(info) => match &info.user {
                Some(user) => (ink::GOOD, format!("{} · {user}", wanted.label())),
                None => (ink::GOOD, format!("{} ouvert", wanted.label())),
            },
            None => (ink::BAD, format!("{} fermé", wanted.label())),
        },
    }
}

/// One line about the running presence.
fn live_summary(snapshot: &Snapshot) -> String {
    match snapshot.phase {
        Phase::Live => {
            let mut parts = vec![format!(
                "en ligne par le {}",
                snapshot.transport.map_or("…", |transport| transport.label())
            )];
            if let Some(since) = snapshot.live_since {
                parts.push(format!("depuis {}", preview::clock(since.elapsed().as_secs())));
            }
            parts.join(" · ")
        }
        Phase::Failing => format!(
            "échec : {}",
            snapshot.error.as_deref().unwrap_or("Discord ne répond pas")
        ),
        Phase::Paused => "en pause".to_owned(),
        Phase::Disabled => "désactivée dans la config".to_owned(),
        Phase::Waiting | Phase::Connecting => "connexion à Discord…".to_owned(),
    }
}
