//! The profiles screen, the background screen and a small text prompt.

use std::io;

use super::app::{App, IDLE, Tone, card_title, warm_names};
use super::input::Input;
use super::logo;
use super::paint::{Canvas, Style, clip, ink};
use super::preview;
use super::term::Key;
use crate::config::{self, Settings};
use crate::{background, engine, profiles};

/// A row of the profiles screen.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Shelf {
    Save,
    Profile(String),
}

/// A row of the background screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Switch {
    Start,
    Stop,
    Autostart,
    Back,
}

impl App {
    /// Asks for a line of text; `None` when the user backs out.
    pub(super) fn ask_text(&mut self, title: &str, question: &str, initial: &str) -> io::Result<Option<String>> {
        let mut input = Input::default();
        input.set(initial);
        loop {
            let mut canvas = self.term.canvas();
            logo::header(&mut canvas, self.theme, title);
            let x = 4;
            let room = (canvas.w - 8).min(60);
            canvas.text(x, 5, question, Style::fg(ink::TEXT).bold());
            canvas.text(x, 7, "›", Style::fg(self.theme.accent()).bold());
            let visible = (room - 4).max(8) as usize;
            let start = input.cursor.saturating_sub(visible - 1);
            let shown: String = input.chars.iter().skip(start).take(visible).collect();
            canvas.text(x + 2, 7, &shown, Style::fg(ink::TEXT));
            for i in 0..room - 2 {
                canvas.put(x + 2 + i, 8, '─', Style::fg(self.theme.at(i as f32 / room as f32).shade(0.45)));
            }
            logo::footer(&mut canvas, &[("Entrée", "valider"), ("Échap", "annuler")]);
            let cursor = (x + 2 + (input.cursor - start) as i32, 7);
            self.term.present(&canvas, Some(cursor))?;
            match self.term.key(IDLE)? {
                Some(Key::Enter) => return Ok(Some(input.value())),
                Some(Key::Esc | Key::Ctrl('c')) => return Ok(None),
                Some(other) => {
                    input.edit(&other);
                }
                None => {}
            }
        }
    }

    // --- Profiles -----------------------------------------------------------

    /// Saved configurations: save the current one, load one, delete one.
    pub(super) fn profiles(&mut self) -> io::Result<()> {
        let mut selected = 0;
        loop {
            let saved = profiles::list(&self.path);
            let mut rows = vec![Shelf::Save];
            rows.extend(saved.iter().map(|(name, _)| Shelf::Profile(name.clone())));
            selected = selected.min(rows.len() - 1);
            let mut canvas = self.term.canvas();
            self.draw_profiles(&mut canvas, &rows, &saved, selected);
            self.term.present(&canvas, None)?;
            let Some(key) = self.term.key(IDLE)? else {
                continue;
            };
            let count = rows.len();
            match key {
                Key::Up | Key::BackTab => selected = (selected + count - 1) % count,
                Key::Down | Key::Tab => selected = (selected + 1) % count,
                Key::Esc | Key::Char('q' | 'Q') | Key::Ctrl('c') => return Ok(()),
                Key::Enter | Key::Char(' ') => match &rows[selected] {
                    Shelf::Save => self.save_profile()?,
                    Shelf::Profile(name) => {
                        let name = name.clone();
                        self.load_profile(&name)?;
                    }
                },
                Key::Delete | Key::Char('x' | 'X') => {
                    if let Shelf::Profile(name) = &rows[selected] {
                        let name = name.clone();
                        let question = format!("Supprimer le profil « {name} » ?");
                        if self.confirm("Profils", &[question.as_str()], "Supprimer", "Garder")? {
                            match profiles::delete(&self.path, &name) {
                                Ok(()) => self.say(Tone::Good, format!("Profil « {name} » supprimé")),
                                Err(error) => self.say(Tone::Bad, error),
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn save_profile(&mut self) -> io::Result<()> {
        let Some(settings) = self.settings.clone() else {
            self.say(Tone::Warn, "Rien à enregistrer : configure d'abord ta carte");
            return Ok(());
        };
        let Some(name) = self.ask_text("Profils", "Nom du nouveau profil", "")? else {
            return Ok(());
        };
        match profiles::save(&self.path, &name, &settings) {
            Ok(name) => self.say(Tone::Good, format!("Profil « {name} » enregistré")),
            Err(error) => self.say(Tone::Bad, error),
        }
        Ok(())
    }

    fn load_profile(&mut self, name: &str) -> io::Result<()> {
        let question = format!("Charger « {name} » à la place de la configuration actuelle ?");
        if !self.confirm("Profils", &[question.as_str()], "Charger", "Annuler")? {
            return Ok(());
        }
        let loaded = profiles::load(&self.path, name, self.settings.as_ref())
            .and_then(|profile| profile.save(&self.path).map(|()| profile));
        match loaded {
            Ok(profile) => {
                if self.worker.is_none() {
                    engine::set_client(profile.client());
                }
                warm_names(&profile);
                self.settings = Some(profile);
                let note = if self.worker.is_some() {
                    format!("Profil « {name} » chargé : la présence suit")
                } else {
                    format!("Profil « {name} » chargé")
                };
                self.say(Tone::Good, note);
            }
            Err(error) => self.say(Tone::Bad, error),
        }
        Ok(())
    }

    fn draw_profiles(
        &self,
        canvas: &mut Canvas,
        rows: &[Shelf],
        saved: &[(String, Result<Settings, String>)],
        selected: usize,
    ) {
        logo::header(canvas, self.theme, "Profils");
        let x = 4;
        let block = (canvas.w - 8).min(90);
        let accent = self.theme.accent();
        canvas.text(
            x,
            4,
            &clip(
                "Tes configurations enregistrées : charger un profil remplace la configuration actuelle.",
                block,
            ),
            Style::fg(ink::MUTED),
        );
        let mut y = 6;
        for (index, row) in rows.iter().enumerate() {
            let chosen = index == selected;
            if chosen {
                canvas.fill(x - 1, y, block + 2, 1, accent.shade(0.8));
                canvas.put(x - 1, y, '▌', Style::fg(accent));
            }
            let strong = if chosen { Style::fg(ink::TEXT).bold() } else { Style::fg(ink::MUTED) };
            let soft = Style::fg(if chosen { ink::MUTED } else { ink::FAINT });
            match row {
                Shelf::Save => {
                    canvas.text(x + 2, y, "+", Style::fg(accent));
                    canvas.text(x + 5, y, "Enregistrer la configuration actuelle", strong);
                    y += 2;
                    if saved.is_empty() {
                        canvas.text(x + 5, y, "Aucun profil pour l'instant.", Style::fg(ink::FAINT));
                    }
                    continue;
                }
                Shelf::Profile(name) => {
                    let content = saved.iter().find(|(each, _)| each == name).map(|(_, content)| content);
                    let (dot, about) = match content {
                        Some(Ok(settings)) => {
                            let count = settings.activities.len();
                            let first = settings.activities.first();
                            let mut about = format!("{count} carte{}", if count > 1 { "s" } else { "" });
                            if let Some(card) = first {
                                about.push_str(&format!(
                                    "  ·  {} {}",
                                    preview::verb(config::kind(&card.status)),
                                    card_title(&card.name, card.app_id)
                                ));
                                if !card.details.trim().is_empty() {
                                    about.push_str(&format!("  ·  « {} »", card.details.trim()));
                                }
                            }
                            (ink::GOOD, about)
                        }
                        Some(Err(error)) => (ink::BAD, format!("illisible : {error}")),
                        None => (ink::FAINT, String::new()),
                    };
                    canvas.put(x + 2, y, '●', Style::fg(dot));
                    canvas.text(x + 5, y, &clip(name, 22), strong);
                    canvas.text(x + 29, y, &clip(&about, block - 31), soft);
                }
            }
            y += 1;
        }
        let mut hints = vec![("↑↓", "choisir")];
        match rows.get(selected) {
            Some(Shelf::Profile(_)) => {
                hints.push(("Entrée", "charger"));
                hints.push(("Suppr", "supprimer"));
            }
            _ => hints.push(("Entrée", "enregistrer")),
        }
        hints.push(("Échap", "retour"));
        logo::footer(canvas, &hints);
    }

    // --- Background ---------------------------------------------------------

    /// The background mode: start it (the window then closes), stop it, start with Windows.
    /// Returns `true` when the window should close.
    pub(super) fn background(&mut self) -> io::Result<bool> {
        let mut selected = 0;
        loop {
            let present = background::executable().is_some();
            let running = background::running();
            let autostart = background::autostart_here();
            let mut rows = Vec::new();
            if present {
                rows.push(if running { Switch::Stop } else { Switch::Start });
                rows.push(Switch::Autostart);
            }
            rows.push(Switch::Back);
            selected = selected.min(rows.len() - 1);
            let mut canvas = self.term.canvas();
            self.draw_background(&mut canvas, &rows, selected, present, running, autostart);
            self.term.present(&canvas, None)?;
            let Some(key) = self.term.key(IDLE)? else {
                continue;
            };
            let count = rows.len();
            let row = rows[selected];
            match key {
                Key::Up | Key::BackTab => selected = (selected + count - 1) % count,
                Key::Down | Key::Tab => selected = (selected + 1) % count,
                Key::Esc | Key::Char('q' | 'Q') | Key::Ctrl('c') => return Ok(false),
                Key::Left | Key::Right if row == Switch::Autostart => self.toggle_autostart(autostart),
                Key::Enter | Key::Char(' ') => match row {
                    Switch::Start => {
                        if self.go_background()? {
                            return Ok(true);
                        }
                    }
                    Switch::Stop => {
                        match self.busy("Arrière-plan", "Arrêt du mode arrière-plan et retrait des cartes…", background::stop)? {
                            Ok(()) => self.say(Tone::Good, "Mode arrière-plan arrêté : cartes retirées"),
                            Err(error) => self.say(Tone::Bad, error),
                        }
                    }
                    Switch::Autostart => self.toggle_autostart(autostart),
                    Switch::Back => return Ok(false),
                },
                _ => {}
            }
        }
    }

    fn toggle_autostart(&mut self, now: bool) {
        match background::set_autostart(!now) {
            Ok(()) if now => self.say(Tone::Good, "Spider ne se lancera plus avec Windows"),
            Ok(()) => self.say(Tone::Good, "Spider se lancera en arrière-plan avec Windows"),
            Err(error) => self.say(Tone::Bad, error),
        }
    }

    /// Hands the presence to the background Spider; `true` when this window can close.
    fn go_background(&mut self) -> io::Result<bool> {
        if self.settings.is_none() {
            self.say(Tone::Warn, "Configure d'abord ta carte");
            return Ok(false);
        }
        // One publisher at a time: this window's presence stops first.
        if let Some(worker) = self.worker.take() {
            self.busy("Arrière-plan", "Passage de la présence à l'arrière-plan…", move || worker.stop())?;
        }
        match self.busy("Arrière-plan", "Démarrage du mode arrière-plan…", background::start)? {
            Ok(()) => Ok(true),
            Err(error) => {
                self.say(Tone::Bad, error);
                Ok(false)
            }
        }
    }

    fn draw_background(
        &self,
        canvas: &mut Canvas,
        rows: &[Switch],
        selected: usize,
        present: bool,
        running: bool,
        autostart: bool,
    ) {
        logo::header(canvas, self.theme, "Arrière-plan");
        let x = 4;
        let block = (canvas.w - 8).min(86);
        let accent = self.theme.accent();
        canvas.text(x, 4, "En arrière-plan, Spider garde ta présence sans aucune fenêtre.", Style::fg(ink::TEXT));
        canvas.text(
            x,
            5,
            &clip(
                "Tu peux fermer cette fenêtre : la carte reste. Rouvrir Spider te rend la main.",
                block,
            ),
            Style::fg(ink::MUTED),
        );
        let (colour, state) = if running {
            (ink::GOOD, "En marche")
        } else {
            (ink::FAINT, "Arrêté")
        };
        canvas.put(x, 7, '●', Style::fg(colour));
        let after = canvas.text(x + 2, 7, state, Style::fg(colour).bold());
        let autostart_text = if autostart {
            "se lance avec Windows"
        } else if background::autostart().is_some() {
            "Windows lance une autre copie de Spider"
        } else {
            "ne se lance pas avec Windows"
        };
        canvas.text(after + 3, 7, &format!("·  {autostart_text}"), Style::fg(ink::MUTED));
        let mut y = 9;
        if !present {
            canvas.put(x, y, '●', Style::fg(ink::BAD));
            canvas.text(
                x + 2,
                y,
                &clip("SpiderBackground.exe est introuvable à côté de Spider.exe : relance build.bat", block),
                Style::fg(ink::BAD),
            );
            y += 2;
        }
        for (index, row) in rows.iter().enumerate() {
            let chosen = index == selected;
            if chosen {
                canvas.fill(x - 1, y, block + 2, 1, accent.shade(0.8));
                canvas.put(x - 1, y, '▌', Style::fg(accent));
            }
            let strong = if chosen { Style::fg(ink::TEXT).bold() } else { Style::fg(ink::MUTED) };
            let soft = Style::fg(if chosen { ink::MUTED } else { ink::FAINT });
            let (label, hint) = match row {
                Switch::Start => ("Passer en arrière-plan", "ferme cette fenêtre, la présence continue"),
                Switch::Stop => ("Arrêter l'arrière-plan", "retire les cartes de ton profil"),
                Switch::Autostart => ("Lancer avec Windows", ""),
                Switch::Back => ("Retour", ""),
            };
            canvas.text(x + 2, y, label, strong);
            if *row == Switch::Autostart {
                let shown = if autostart { "‹ Oui ›" } else { "‹ Non ›" };
                let colour = if chosen { accent.light(0.3) } else { ink::TEXT };
                let end = canvas.text(x + 28, y, shown, Style::fg(colour).bold());
                canvas.text(end + 2, y, "à la connexion, sans fenêtre", soft);
            } else {
                canvas.text(x + 28, y, &clip(hint, block - 30), soft);
            }
            y += 1;
        }
        let mut hints = vec![("↑↓", "choisir"), ("Entrée", "valider")];
        if rows.get(selected) == Some(&Switch::Autostart) {
            hints.push(("←→", "changer"));
        }
        hints.push(("Échap", "retour"));
        logo::footer(canvas, &hints);
    }
}
