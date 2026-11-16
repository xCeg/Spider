//! Configuration: the hub (cards and settings) and the card wizard, one question at a time,
//! with the card drawn live on the right as it is typed.

use std::io;
use std::time::Instant;

use super::app::{App, FRAME, IDLE, Tone, card_title, view_of};
use super::logo;
use super::paint::{Canvas, Style, clip, ink, width};
use super::preview::{self, CardView};
use super::input::Input;
use super::term::Key;
use crate::config::{self, Activity, Button, PLATFORMS, STATUSES, Settings, TIMER_MODES, TRANSPORTS};
use crate::engine::{self, Client, TimerMode};

/// The corner pictures of the Spotify, YouTube and Twitch templates (Cinnabug's uploads).
const SPOTIFY_ICON: &str = "https://files.catbox.moe/o1koxy.png";
const YOUTUBE_BADGE: &str = "https://files.catbox.moe/14h5ok.png";
const TWITCH_BADGE: &str = "https://files.catbox.moe/bcof2b.png";

/// A ready-made start: the application and the type, the rest stays free.
struct Template {
    name: &'static str,
    hint: &'static str,
    /// The card's title when the application's own name is not the one to show.
    title: &'static str,
    app_id: u64,
    status: &'static str,
    small_image: &'static str,
    small_text: &'static str,
    stream_url: &'static str,
    button: Option<(&'static str, &'static str)>,
}

const TEMPLATES: [Template; 6] = [
    Template {
        name: "Ma carte",
        hint: "ton application Discord, tout est libre",
        title: "",
        app_id: 0,
        status: "playing",
        small_image: "",
        small_text: "",
        stream_url: "",
        button: None,
    },
    Template {
        name: "Spotify",
        hint: "« Écoute Spotify » avec la barre de lecture",
        title: "",
        app_id: engine::SPOTIFY_APP_ID,
        status: "listening",
        small_image: SPOTIFY_ICON,
        small_text: "Spotify",
        stream_url: "",
        button: None,
    },
    Template {
        name: "YouTube",
        hint: "« Regarde YouTube » et un bouton Watch",
        title: "YouTube",
        // Discord calls this application "Watch Together" (its YouTube activity): the card
        // is titled YouTube on the debug port; the pipe keeps the application's own name.
        app_id: 880_218_394_199_220_334,
        status: "watching",
        small_image: YOUTUBE_BADGE,
        small_text: "YouTube",
        stream_url: "",
        button: Some(("Watch", "https://www.youtube.com/")),
    },
    Template {
        name: "Twitch",
        hint: "« Streame » en direct (port de debug)",
        title: "Twitch",
        app_id: 0,
        status: "streaming",
        small_image: TWITCH_BADGE,
        small_text: "Twitch",
        stream_url: "https://www.twitch.tv/",
        button: None,
    },
    Template {
        name: "Crunchyroll",
        hint: "« Regarde Crunchyroll »",
        title: "",
        app_id: 981_509_069_309_354_054,
        status: "watching",
        small_image: "",
        small_text: "",
        stream_url: "",
        button: None,
    },
    Template {
        name: "League of Legends",
        hint: "« Joue à League of Legends »",
        title: "",
        app_id: 401_518_684_763_586_560,
        status: "playing",
        small_image: "",
        small_text: "",
        stream_url: "",
        button: None,
    },
];

/// One question of the wizard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Field {
    Template,
    AppId,
    Status,
    Name,
    Details,
    Rotation,
    RotationSeconds,
    State,
    Party,
    Image,
    ImageText,
    SmallImage,
    SmallText,
    StreamUrl,
    Platform,
    Label1,
    Link1,
    Label2,
    Link2,
    Timer,
    Countdown,
}

const STEPS: [Field; 21] = [
    Field::Template,
    Field::AppId,
    Field::Status,
    Field::Name,
    Field::Details,
    Field::Rotation,
    Field::RotationSeconds,
    Field::State,
    Field::Party,
    Field::Image,
    Field::ImageText,
    Field::SmallImage,
    Field::SmallText,
    Field::StreamUrl,
    Field::Platform,
    Field::Label1,
    Field::Link1,
    Field::Label2,
    Field::Link2,
    Field::Timer,
    Field::Countdown,
];

const STATUS_CHOICES: [(&str, &str); 5] = [
    ("Joue à", "Playing"),
    ("Streame", "Streaming · demande le port de debug"),
    ("Écoute", "Listening"),
    ("Regarde", "Watching"),
    ("Participe à", "Competing"),
];

const PLATFORM_CHOICES: [(&str, &str); 4] = [
    ("Ordinateur", "pas de badge"),
    ("Xbox", "pastille Xbox sur l'image"),
    ("PlayStation", "pastille PS5 sur l'image"),
    ("Meta Quest", "pastille VR sur l'image"),
];

/// The timer modes, then "none" (`config::TIMER_MODES` order).
const TIMER_CHOICES: [(&str, &str); 4] = [
    ("Depuis l'arrivée", "« 12:34 écoulées », remis à zéro au lancement"),
    ("Heure actuelle", "affiche l'heure qu'il est"),
    ("Compte à rebours", "« 12:34 restantes », puis recommence"),
    ("Aucun", "pas de chronomètre"),
];

/// How often the first line changes, in seconds.
const ROTATIONS: [u32; 4] = [15, 30, 60, 300];
const ROTATION_CHOICES: [(&str, &str); 4] = [
    ("15 secondes", "le plus rapide que Discord suit"),
    ("30 secondes", ""),
    ("1 minute", ""),
    ("5 minutes", ""),
];

impl Field {
    fn title(self) -> &'static str {
        match self {
            Field::Template => "Point de départ",
            Field::AppId => "Application Discord",
            Field::Status => "Type d'activité",
            Field::Name => "Titre de la carte",
            Field::Details => "Première ligne",
            Field::Rotation => "Rotation de la première ligne",
            Field::RotationSeconds => "Vitesse de la rotation",
            Field::State => "Deuxième ligne",
            Field::Party => "Groupe",
            Field::Image => "Grande image",
            Field::ImageText => "Texte de la grande image",
            Field::SmallImage => "Petite image",
            Field::SmallText => "Texte de la petite image",
            Field::StreamUrl => "Lien du stream",
            Field::Platform => "Plateforme",
            Field::Label1 => "Bouton 1 · texte",
            Field::Link1 => "Bouton 1 · lien",
            Field::Label2 => "Bouton 2 · texte",
            Field::Link2 => "Bouton 2 · lien",
            Field::Timer => "Chronomètre",
            Field::Countdown => "Durée du compte à rebours",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Field::Template => {
                "Un modèle remplit l'application et le type pour toi ; tout reste modifiable ensuite."
            }
            Field::AppId => {
                "L'« Application ID » de ton application sur discord.com/developers/applications. Son nom devient le titre de la carte."
            }
            Field::Status => "Le mot que Discord écrit devant le nom de l'application.",
            Field::Name => "Laisse vide pour garder le nom de l'application.",
            Field::Details => {
                "Le texte sous le titre. Vide = rien. {heure}, {date} et {jour} se remplacent tout seuls."
            }
            Field::Rotation => {
                "D'autres premières lignes, affichées à tour de rôle avec celle d'avant. Sépare-les par des points-virgules ; vide = pas de rotation."
            }
            Field::RotationSeconds => "Combien de temps chaque première ligne reste affichée.",
            Field::State => {
                "Une ligne de plus, sous la première. Vide = rien. {heure}, {date} et {jour} marchent aussi."
            }
            Field::Party => {
                "« (2 sur 4) » après la deuxième ligne : la taille du groupe, une barre, le maximum. Vide = rien."
            }
            Field::Image => {
                "La clé d'une image de ton application (Rich Presence > Art Assets) ou un lien https:// vers une image. Vide = pas d'image."
            }
            Field::ImageText => "Le texte qui apparaît au survol de la grande image.",
            Field::SmallImage => {
                "La pastille dans le coin de la grande image : une clé ou un lien https://. Vide = pas de pastille."
            }
            Field::SmallText => "Le texte qui apparaît au survol de la pastille.",
            Field::StreamUrl => {
                "Un lien Twitch ou YouTube : c'est lui qui donne le statut « en direct »."
            }
            Field::Platform => {
                "Une pastille de console sur l'image (si tu n'as pas mis de petite image), et son logo à côté du nom par le port de debug."
            }
            Field::Label1 => "Le texte du premier bouton, 32 caractères au plus. Vide = pas de bouton.",
            Field::Link1 => "Où mène le bouton : un lien https://.",
            Field::Label2 => "Un second bouton. Vide = un seul bouton.",
            Field::Link2 => "Où mène le second bouton : un lien https://.",
            Field::Timer => "Ce que montre le chronomètre sous la carte.",
            Field::Countdown => {
                "En minutes, de 1 à 1440 (24 heures). Il recommence quand il arrive à zéro."
            }
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Field::AppId => "ex. 123456789012345678",
            Field::Name => "vide = le nom de l'application",
            Field::Details => "ex. En train de coder",
            Field::Rotation => "ex. Je teste ; Je fais une pause",
            Field::State => "ex. Il est {heure}",
            Field::Party => "ex. 2/4",
            Field::Image | Field::SmallImage => "ex. logo   ou   https://exemple.com/image.png",
            Field::ImageText | Field::SmallText => "ex. Spider",
            Field::StreamUrl => "https://www.twitch.tv/ton_pseudo",
            Field::Label1 | Field::Label2 => "ex. Mon site",
            Field::Link1 | Field::Link2 => "https://",
            Field::Countdown => "ex. 30",
            _ => "",
        }
    }

    fn choices(self) -> Option<Vec<(&'static str, &'static str)>> {
        match self {
            Field::Template => Some(TEMPLATES.iter().map(|t| (t.name, t.hint)).collect()),
            Field::Status => Some(STATUS_CHOICES.to_vec()),
            Field::Platform => Some(PLATFORM_CHOICES.to_vec()),
            Field::Timer => Some(TIMER_CHOICES.to_vec()),
            Field::RotationSeconds => Some(ROTATION_CHOICES.to_vec()),
            _ => None,
        }
    }

    /// The longest text Discord takes.
    fn max(self) -> usize {
        match self {
            Field::Label1 | Field::Label2 => 32,
            Field::Link1 | Field::Link2 | Field::StreamUrl | Field::Image | Field::SmallImage => 512,
            Field::Rotation => 600,
            Field::AppId => 24,
            Field::Party => 15,
            Field::Countdown => 4,
            _ => 128,
        }
    }

    fn optional(self) -> bool {
        !matches!(self, Field::AppId | Field::Link1 | Field::Link2 | Field::Countdown)
    }

    /// Whether the picture keys of the application can complete this answer.
    fn picture(self) -> bool {
        matches!(self, Field::Image | Field::SmallImage)
    }

    /// Whether the question makes sense for this card.
    fn shown(self, card: &Activity, new_card: bool) -> bool {
        let label = |index: usize| {
            card.buttons
                .get(index)
                .is_some_and(|button| !button.label.trim().is_empty())
        };
        let link = |index: usize| {
            card.buttons
                .get(index)
                .is_some_and(|button| config::is_url(button.url.trim()))
        };
        match self {
            Field::Template => new_card,
            Field::RotationSeconds => !card.rotation.is_empty(),
            Field::ImageText => !card.image.trim().is_empty(),
            Field::SmallText => !card.small_image.trim().is_empty(),
            Field::StreamUrl => config::kind(&card.status) == 1,
            Field::Link1 => label(0),
            Field::Label2 => label(0) && link(0),
            Field::Link2 => label(1),
            Field::Countdown => card.timer && matches!(card.timer_mode(), TimerMode::Countdown(_)),
            _ => true,
        }
    }

    fn check(self, value: &str) -> Result<(), String> {
        if value.chars().count() > self.max() {
            return Err(format!("{} caractères au maximum", self.max()));
        }
        match self {
            Field::AppId => config::parse_app_id(value).map(|_| ()),
            Field::Image | Field::SmallImage => {
                if value.contains("://") && !config::is_url(value) {
                    Err("un lien d'image commence par https://".to_owned())
                } else if !value.contains("://") && value.contains(char::is_whitespace) {
                    Err("une clé d'image n'a pas d'espace (ou colle un lien https://)".to_owned())
                } else {
                    Ok(())
                }
            }
            Field::StreamUrl if !value.is_empty() && !config::is_url(value) => {
                Err("un lien commence par https://".to_owned())
            }
            Field::Link1 | Field::Link2 if !config::is_url(value) => {
                Err("le bouton a besoin d'un lien https://".to_owned())
            }
            Field::Rotation => match lines(value).iter().find(|line| line.chars().count() > 128) {
                Some(_) => Err("chaque ligne fait 128 caractères au plus".to_owned()),
                None => Ok(()),
            },
            Field::Party if !value.is_empty() && party(value).is_none() => {
                Err("écris la taille puis le maximum, par exemple 2/4".to_owned())
            }
            Field::Countdown => match value.parse::<u32>() {
                Ok(1..=1440) => Ok(()),
                _ => Err("un nombre de minutes, de 1 à 1440".to_owned()),
            },
            _ => Ok(()),
        }
    }
}

/// The rotation's lines, from "one ; two ; three".
fn lines(text: &str) -> Vec<String> {
    text.split(';')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// "2/4" (or "2 / 4", "2 sur 4") to (2, 4), when the group can exist.
fn party(text: &str) -> Option<(u32, u32)> {
    let text = text.replace("sur", "/");
    let (size, max) = text.split_once('/')?;
    let (size, max) = (size.trim().parse().ok()?, max.trim().parse().ok()?);
    (1..=max).contains(&size).then_some((size, max))
}

fn button(card: &mut Activity, index: usize) -> &mut Button {
    while card.buttons.len() <= index {
        card.buttons.push(Button::default());
    }
    &mut card.buttons[index]
}

fn text_of(card: &Activity, field: Field) -> String {
    let button = |index: usize| card.buttons.get(index).cloned().unwrap_or_default();
    match field {
        Field::AppId if card.app_id != 0 => card.app_id.to_string(),
        Field::Name => card.name.clone(),
        Field::Details => card.details.clone(),
        Field::Rotation => card.rotation.join(" ; "),
        Field::State => card.state.clone(),
        Field::Party => card
            .party_size()
            .map(|(size, max)| format!("{size}/{max}"))
            .unwrap_or_default(),
        Field::Image => card.image.clone(),
        Field::ImageText => card.image_text.clone(),
        Field::SmallImage => card.small_image.clone(),
        Field::SmallText => card.small_text.clone(),
        Field::StreamUrl => card.stream_url.clone(),
        Field::Label1 => button(0).label,
        Field::Link1 => button(0).url,
        Field::Label2 => button(1).label,
        Field::Link2 => button(1).url,
        Field::Countdown => card.countdown_minutes.clamp(1, 1440).to_string(),
        _ => String::new(),
    }
}

fn set_text(card: &mut Activity, field: Field, value: String) {
    match field {
        Field::AppId => card.app_id = config::parse_app_id(&value).unwrap_or(0),
        Field::Name => card.name = value,
        Field::Details => card.details = value,
        Field::Rotation => card.rotation = lines(&value),
        Field::State => card.state = value,
        Field::Party => {
            card.party = party(&value).map_or_else(Vec::new, |(size, max)| vec![size, max]);
        }
        Field::Image => card.image = value,
        Field::ImageText => card.image_text = value,
        Field::SmallImage => card.small_image = value,
        Field::SmallText => card.small_text = value,
        Field::StreamUrl => card.stream_url = value,
        Field::Label1 => button(card, 0).label = value,
        Field::Link1 => button(card, 0).url = value,
        Field::Label2 => button(card, 1).label = value,
        Field::Link2 => button(card, 1).url = value,
        Field::Countdown => card.countdown_minutes = value.parse().unwrap_or(30),
        _ => {}
    }
}

fn choice_of(card: &Activity, field: Field) -> usize {
    match field {
        Field::Status => STATUSES
            .iter()
            .position(|status| *status == card.status.trim().to_ascii_lowercase())
            .unwrap_or(0),
        Field::Platform => {
            let platform = config::platform(&card.platform).unwrap_or("");
            PLATFORMS
                .iter()
                .position(|each| *each == platform)
                .unwrap_or(0)
        }
        Field::Timer if !card.timer => TIMER_CHOICES.len() - 1,
        Field::Timer => match card.timer_mode() {
            TimerMode::Start => 0,
            TimerMode::Clock => 1,
            TimerMode::Countdown(_) => 2,
        },
        Field::RotationSeconds => ROTATIONS
            .iter()
            .position(|seconds| *seconds >= card.rotation_seconds)
            .unwrap_or(ROTATIONS.len() - 1),
        _ => 0,
    }
}

fn set_choice(card: &mut Activity, field: Field, index: usize) {
    match field {
        Field::Template => {
            let template = &TEMPLATES[index.min(TEMPLATES.len() - 1)];
            if template.app_id != 0 {
                card.app_id = template.app_id;
            }
            card.status = template.status.to_owned();
            card.name = template.title.to_owned();
            card.small_image = template.small_image.to_owned();
            card.small_text = template.small_text.to_owned();
            card.stream_url = template.stream_url.to_owned();
            card.buttons = template
                .button
                .map(|(label, url)| {
                    vec![Button {
                        label: label.to_owned(),
                        url: url.to_owned(),
                    }]
                })
                .unwrap_or_default();
        }
        Field::Status => card.status = STATUSES[index.min(STATUSES.len() - 1)].to_owned(),
        Field::Platform => card.platform = PLATFORMS[index.min(PLATFORMS.len() - 1)].to_owned(),
        Field::Timer => {
            card.timer = index < TIMER_MODES.len();
            if card.timer {
                card.timer_mode = TIMER_MODES[index].to_owned();
            }
        }
        Field::RotationSeconds => card.rotation_seconds = ROTATIONS[index.min(ROTATIONS.len() - 1)],
        _ => {}
    }
}

/// Removes half-made buttons and texts the questions skipped.
fn tidy(card: &mut Activity) {
    card.buttons.retain(|button| {
        !button.label.trim().is_empty() && config::is_url(button.url.trim())
    });
    for text in [
        &mut card.name,
        &mut card.details,
        &mut card.state,
        &mut card.image,
        &mut card.image_text,
        &mut card.small_image,
        &mut card.small_text,
        &mut card.stream_url,
    ] {
        *text = text.trim().to_owned();
    }
    if card.image.is_empty() {
        card.image_text.clear();
    }
    if card.small_image.is_empty() {
        card.small_text.clear();
    }
    if config::kind(&card.status) != 1 {
        card.stream_url.clear();
    }
}

/// The wizard's state.
struct Wizard {
    card: Activity,
    new_card: bool,
    number: usize,
    step: usize,
    input: Input,
    choice: usize,
    error: Option<String>,
    note: Option<(Tone, String)>,
    /// An ID the user keeps although it could not be checked (no Internet).
    unchecked: Option<u64>,
    /// When the current question showed (for its entrance).
    since: Instant,
    /// Tab through the application's pictures: what was typed, and the picture shown.
    completion: Option<(String, usize)>,
}

impl Wizard {
    fn field(&self) -> Field {
        STEPS[self.step]
    }

    fn shown(&self) -> Vec<usize> {
        (0..STEPS.len())
            .filter(|index| STEPS[*index].shown(&self.card, self.new_card))
            .collect()
    }

    fn load(&mut self) {
        let field = self.field();
        if field.choices().is_some() {
            self.choice = choice_of(&self.card, field);
        } else {
            self.input.set(&text_of(&self.card, field));
        }
        self.error = None;
        self.completion = None;
        self.since = Instant::now();
        // The application's picture keys, fetched while the question is read.
        let id = self.card.app_id;
        if field.picture() && id != 0 && engine::known_application_assets(id).is_none() {
            std::thread::spawn(move || {
                let _ = engine::application_assets(id);
            });
        }
    }

    /// The application's pictures that match what was typed; `None` while they are unknown
    /// (or when the answer is a link).
    fn pictures(&self) -> Option<Vec<String>> {
        if !self.field().picture() || self.card.app_id == 0 {
            return None;
        }
        let all = engine::known_application_assets(self.card.app_id)?;
        let typed = self
            .completion
            .as_ref()
            .map_or_else(|| self.input.value(), |(typed, _)| typed.clone());
        if typed.contains("://") {
            return Some(Vec::new());
        }
        let typed = typed.trim().to_lowercase();
        Some(
            all.into_iter()
                .filter(|name| name.to_lowercase().contains(&typed))
                .collect(),
        )
    }

    /// Tab: the next picture that matches what was typed; `false` when there is none.
    fn complete(&mut self) -> bool {
        let Some(found) = self.pictures().filter(|found| !found.is_empty()) else {
            return false;
        };
        let typed = self
            .completion
            .as_ref()
            .map_or_else(|| self.input.value(), |(typed, _)| typed.clone());
        let next = match &self.completion {
            Some((_, index)) => (index + 1) % found.len(),
            None => 0,
        };
        self.input.set(&found[next]);
        self.completion = Some((typed, next));
        self.error = None;
        true
    }

    /// Moves to the next question; `false` when there is none.
    fn forward(&mut self) -> bool {
        match (self.step + 1..STEPS.len()).find(|index| STEPS[*index].shown(&self.card, self.new_card)) {
            Some(next) => {
                self.step = next;
                self.load();
                true
            }
            None => false,
        }
    }

    /// Moves to the previous question; `false` at the first one.
    fn back(&mut self) -> bool {
        match (0..self.step).rev().find(|index| STEPS[*index].shown(&self.card, self.new_card)) {
            Some(previous) => {
                self.step = previous;
                self.load();
                self.note = None;
                true
            }
            None => false,
        }
    }

    /// The card as it would look with the question's current answer.
    fn view(&self) -> CardView {
        let mut card = self.card.clone();
        let field = self.field();
        if field.choices().is_some() {
            set_choice(&mut card, field, self.choice);
        } else if field != Field::AppId {
            set_text(&mut card, field, self.input.value());
        }
        view_of(&card)
    }
}

/// A row of the configuration hub.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Row {
    Card(usize),
    Add,
    Transport,
    Client,
    Refresh,
    Enabled,
    AutoStart,
    Save,
    Cancel,
}

const REFRESHES: [f64; 5] = [5.0, 10.0, 15.0, 30.0, 60.0];

fn rows(draft: &Settings) -> Vec<Row> {
    let mut list: Vec<Row> = (0..draft.activities.len()).map(Row::Card).collect();
    if draft.activities.len() < 3 {
        list.push(Row::Add);
    }
    list.extend([
        Row::Transport,
        Row::Client,
        Row::Refresh,
        Row::Enabled,
        Row::AutoStart,
        Row::Save,
        Row::Cancel,
    ]);
    list
}

fn transport_label(word: &str) -> (&'static str, &'static str) {
    match word {
        "cdp" => ("Port de debug", "Streaming, badges, plusieurs cartes"),
        "rpc" => ("Pipe", "le plus simple : rien à relancer"),
        _ => ("Auto", "port de debug si ouvert, sinon pipe"),
    }
}

/// The next value in a list, wrapping, `step` = +1 or -1.
fn cycle(index: usize, count: usize, step: i32) -> usize {
    ((index as i32 + step).rem_euclid(count as i32)) as usize
}

impl App {
    /// The configuration hub: the cards, the settings, Save. A first start goes straight to
    /// the wizard for the first card.
    pub(super) fn configure(&mut self) -> io::Result<()> {
        let original = self.settings.clone();
        let mut draft = original.clone().unwrap_or_default();
        draft.theme = self.theme.word.to_owned();
        if draft.activities.is_empty() {
            match self.wizard(None, 1)? {
                Some(card) => draft.activities.push(card),
                None => return Ok(()),
            }
        }
        let mut selected = 0;
        loop {
            let list = rows(&draft);
            selected = selected.min(list.len() - 1);
            let mut canvas = self.term.canvas();
            self.draw_hub(&mut canvas, &draft, &list, selected, original.as_ref());
            self.term.present(&canvas, None)?;
            let Some(key) = self.term.key(IDLE)? else {
                continue;
            };
            let row = list[selected];
            match key {
                Key::Up | Key::BackTab => selected = cycle(selected, list.len(), -1),
                Key::Down | Key::Tab => selected = cycle(selected, list.len(), 1),
                Key::Left | Key::Right => {
                    let step = if key == Key::Left { -1 } else { 1 };
                    match row {
                        Row::Transport => {
                            let index = TRANSPORTS
                                .iter()
                                .position(|word| *word == draft.transport.trim().to_ascii_lowercase())
                                .unwrap_or(0);
                            draft.transport = TRANSPORTS[cycle(index, TRANSPORTS.len(), step)].to_owned();
                        }
                        Row::Client => {
                            let index = Client::ALL
                                .iter()
                                .position(|client| *client == draft.client())
                                .unwrap_or(0);
                            draft.client = Client::ALL[cycle(index, Client::ALL.len(), step)].word().to_owned();
                        }
                        Row::Refresh => {
                            let index = REFRESHES
                                .iter()
                                .position(|seconds| *seconds >= draft.refresh_seconds)
                                .unwrap_or(REFRESHES.len() - 1);
                            draft.refresh_seconds = REFRESHES[cycle(index, REFRESHES.len(), step)];
                        }
                        Row::Enabled => draft.enabled = !draft.enabled,
                        Row::AutoStart => draft.auto_start = !draft.auto_start,
                        _ => {}
                    }
                }
                Key::Delete | Key::Char('x' | 'X') => {
                    if let Row::Card(index) = row
                        && draft.activities.len() > 1
                    {
                        let question = format!("Retirer la carte {} de la configuration ?", index + 1);
                        if self.confirm("Retirer une carte", &[question.as_str()], "Retirer", "Garder")? {
                            draft.activities.remove(index);
                        }
                    }
                }
                Key::Enter | Key::Char(' ') => match row {
                    Row::Card(index) => {
                        let card = draft.activities[index].clone();
                        if let Some(card) = self.wizard(Some(card), index + 1)? {
                            draft.activities[index] = card;
                        }
                    }
                    Row::Add => {
                        let number = draft.activities.len() + 1;
                        if let Some(card) = self.wizard(None, number)? {
                            draft.activities.push(card);
                            selected = draft.activities.len() - 1;
                        }
                    }
                    Row::Enabled => draft.enabled = !draft.enabled,
                    Row::AutoStart => draft.auto_start = !draft.auto_start,
                    Row::Transport | Row::Client | Row::Refresh => {}
                    Row::Save => {
                        self.save(draft);
                        return Ok(());
                    }
                    Row::Cancel => {
                        if self.leave(&draft, original.as_ref())? {
                            return Ok(());
                        }
                    }
                },
                Key::Ctrl('s') => {
                    self.save(draft);
                    return Ok(());
                }
                Key::Esc | Key::Ctrl('c') if self.leave(&draft, original.as_ref())? => return Ok(()),
                _ => {}
            }
        }
    }

    /// Whether to leave the hub: at once when nothing changed, after a question otherwise.
    fn leave(&mut self, draft: &Settings, original: Option<&Settings>) -> io::Result<bool> {
        if original == Some(draft) {
            return Ok(true);
        }
        self.confirm(
            "Modifications",
            &["Quitter sans enregistrer ?", "Les changements seront perdus."],
            "Quitter",
            "Rester",
        )
    }

    fn save(&mut self, draft: Settings) {
        match draft.save(&self.path) {
            Ok(()) => {
                let first = self.settings.is_none();
                // While the presence runs, its loop sees the client change itself (and
                // clears the old account's cards first); set it here only when it is idle.
                if self.worker.is_none() {
                    engine::set_client(draft.client());
                }
                self.settings = Some(draft);
                if self.worker.is_some() {
                    self.say(Tone::Good, "Enregistré : la présence suit la nouvelle configuration");
                } else if first {
                    self.selected = 0;
                    self.say(Tone::Good, "Tout est prêt : Entrée pour lancer la présence");
                } else {
                    self.say(Tone::Good, "Configuration enregistrée");
                }
            }
            Err(error) => self.say(Tone::Bad, format!("Enregistrement impossible : {error}")),
        }
    }

    fn draw_hub(&self, canvas: &mut Canvas, draft: &Settings, list: &[Row], selected: usize, original: Option<&Settings>) {
        logo::header(canvas, self.theme, "Configuration");
        let wide = canvas.w >= 108;
        let block = if wide { canvas.w - preview::WIDTH - 14 } else { canvas.w - 8 }.min(78);
        let x = 4;
        let accent = self.theme.accent();
        let probe = self.probe.lock().map(|probe| probe.clone()).unwrap_or_default();
        let mut y = 4;
        let section = |canvas: &mut Canvas, y: &mut i32, title: &str| {
            canvas.text(x, *y, title, Style::fg(ink::FAINT).bold());
            *y += 1;
        };
        section(canvas, &mut y, "CARTES");
        for (index, row) in list.iter().enumerate() {
            if *row == Row::Transport {
                y += 1;
                section(canvas, &mut y, "RÉGLAGES");
            }
            if *row == Row::Save {
                y += 1;
            }
            let chosen = index == selected;
            if chosen {
                canvas.fill(x - 1, y, block + 2, 1, accent.shade(0.8));
                canvas.put(x - 1, y, '▌', Style::fg(accent));
            }
            let strong = if chosen { Style::fg(ink::TEXT).bold() } else { Style::fg(ink::MUTED) };
            let soft = Style::fg(if chosen { ink::MUTED } else { ink::FAINT });
            let value = |canvas: &mut Canvas, text: &str, hint: &str| {
                let shown = format!("‹ {text} ›");
                let colour = if chosen { accent.light(0.3) } else { ink::TEXT };
                canvas.text(x + 22, y, &shown, Style::fg(colour).bold());
                canvas.text(x + 22 + width(&shown) + 2, y, &clip(hint, block - 26 - width(&shown)), soft);
            };
            match row {
                Row::Card(card) => {
                    let activity = &draft.activities[*card];
                    canvas.text(x + 2, y, &(card + 1).to_string(), Style::fg(accent));
                    canvas.text(x + 5, y, preview::verb(config::kind(&activity.status)), strong);
                    let mut about = card_title(&activity.name, activity.app_id);
                    if !activity.details.trim().is_empty() {
                        about.push_str(&format!("  ·  « {} »", activity.details.trim()));
                    }
                    canvas.text(x + 18, y, &clip(&about, block - 20), soft);
                }
                Row::Add => {
                    canvas.text(x + 2, y, "+", Style::fg(accent));
                    canvas.text(x + 5, y, "Ajouter une carte", strong);
                }
                Row::Transport => {
                    canvas.text(x + 2, y, "Transport", strong);
                    let (label, hint) = transport_label(draft.transport.trim());
                    value(canvas, label, hint);
                }
                Row::Client => {
                    canvas.text(x + 2, y, "Client Discord", strong);
                    let client = draft.client();
                    let hint = match client {
                        Client::Auto => "le premier Discord ouvert".to_owned(),
                        chosen_client => probe
                            .accounts
                            .iter()
                            .find(|info| info.client == chosen_client)
                            .map_or_else(
                                || "fermé en ce moment".to_owned(),
                                |info| info.user.as_ref().map_or_else(|| "ouvert".to_owned(), |user| format!("compte : {user}")),
                            ),
                    };
                    value(canvas, client.label(), &hint);
                }
                Row::Refresh => {
                    canvas.text(x + 2, y, "Rafraîchir", strong);
                    value(
                        canvas,
                        &format!("{} s", draft.refresh_seconds.round() as u64),
                        "renvoi des cartes inchangées",
                    );
                }
                Row::Enabled => {
                    canvas.text(x + 2, y, "Présence", strong);
                    let (word, hint) = if draft.enabled {
                        ("Activée", "les cartes sont publiées")
                    } else {
                        ("Désactivée", "cartes retirées, fichier gardé")
                    };
                    value(canvas, word, hint);
                }
                Row::AutoStart => {
                    canvas.text(x + 2, y, "Au démarrage", strong);
                    let (word, hint) = if draft.auto_start {
                        ("Lancer la présence", "Spider publie dès qu'il s'ouvre")
                    } else {
                        ("Menu", "Spider attend que tu choisisses")
                    };
                    value(canvas, word, hint);
                }
                Row::Save => {
                    let colour = if chosen { ink::GOOD } else { ink::MUTED };
                    canvas.text(x + 2, y, "Enregistrer", Style::fg(colour).bold());
                    if original != Some(draft) {
                        canvas.text(x + 22, y, "● modifications non enregistrées", Style::fg(ink::WARN));
                    }
                }
                Row::Cancel => {
                    canvas.text(x + 2, y, "Annuler", strong);
                }
            }
            y += 1;
        }
        // The selected card (or the first one) drawn on the right, with the file's warnings.
        if wide {
            let shown = match list[selected] {
                Row::Card(index) => index,
                _ => 0,
            };
            let px = canvas.w - preview::WIDTH - 5;
            canvas.text(px, 4, &format!("APERÇU · CARTE {}", shown + 1), Style::fg(ink::FAINT).bold());
            if let Some(activity) = draft.activities.get(shown) {
                let view = view_of(activity);
                preview::draw(canvas, px, 5, &view, self.theme);
                let first = 6 + preview::height(&view);
                for (wy, warning) in (first..canvas.h - 2).zip(draft.warnings()) {
                    canvas.put(px, wy, '●', Style::fg(ink::WARN));
                    canvas.text(px + 2, wy, &clip(&warning, preview::WIDTH), Style::fg(ink::MUTED));
                }
            }
        }
        let mut hints = vec![("↑↓", "choisir")];
        match list[selected] {
            Row::Card(_) => {
                hints.push(("Entrée", "modifier"));
                if draft.activities.len() > 1 {
                    hints.push(("Suppr", "retirer"));
                }
            }
            Row::Transport | Row::Client | Row::Refresh | Row::Enabled | Row::AutoStart => {
                hints.push(("←→", "changer"))
            }
            _ => hints.push(("Entrée", "valider")),
        }
        hints.push(("Ctrl+S", "enregistrer"));
        hints.push(("Échap", "annuler"));
        logo::footer(canvas, &hints);
    }

    /// Asks the card's questions one after the other; `None` when the user backs out.
    fn wizard(&mut self, start: Option<Activity>, number: usize) -> io::Result<Option<Activity>> {
        let new_card = start.is_none();
        let card = start.unwrap_or_default();
        // Fetch the application's name in the background, for the preview's title.
        if card.app_id != 0 {
            let id = card.app_id;
            std::thread::spawn(move || {
                let _ = engine::application_name(id);
            });
        }
        let mut wizard = Wizard {
            card,
            new_card,
            number,
            step: 0,
            input: Input::default(),
            choice: 0,
            error: None,
            note: None,
            unchecked: None,
            since: Instant::now(),
            completion: None,
        };
        if !Field::Template.shown(&wizard.card, new_card) {
            wizard.forward();
        } else {
            wizard.load();
        }
        loop {
            let mut canvas = self.term.canvas();
            let cursor = self.draw_wizard(&mut canvas, &wizard, false);
            self.term.present(&canvas, cursor)?;
            let entering = wizard.since.elapsed().as_secs_f32() < 0.35;
            let Some(key) = self.term.key(if entering { FRAME } else { IDLE })? else {
                continue;
            };
            let field = wizard.field();
            let choices = field.choices();
            let back = match &key {
                Key::Esc | Key::BackTab => true,
                Key::Up => choices.is_none(),
                _ => false,
            };
            if back {
                if !wizard.back() {
                    return Ok(None);
                }
                continue;
            }
            if let Key::Ctrl('c') = key {
                return Ok(None);
            }
            if let Some(options) = &choices {
                match key {
                    Key::Up | Key::Left => wizard.choice = cycle(wizard.choice, options.len(), -1),
                    Key::Down | Key::Right => wizard.choice = cycle(wizard.choice, options.len(), 1),
                    Key::Char(digit @ '1'..='9') => {
                        let index = digit as usize - '1' as usize;
                        if index < options.len() {
                            wizard.choice = index;
                        }
                    }
                    Key::Enter | Key::Tab | Key::Char(' ') => {
                        set_choice(&mut wizard.card, field, wizard.choice);
                        wizard.note = None;
                        if !wizard.forward() {
                            tidy(&mut wizard.card);
                            return Ok(Some(wizard.card));
                        }
                    }
                    _ => {}
                }
                continue;
            }
            // Tab completes a picture key when the application has some.
            if key == Key::Tab && wizard.complete() {
                continue;
            }
            match key {
                Key::Enter | Key::Down | Key::Tab => {
                    let value = wizard.input.value().trim().to_owned();
                    if let Err(error) = field.check(&value) {
                        wizard.error = Some(error);
                        continue;
                    }
                    if field == Field::AppId && !self.check_app(&mut wizard, &value)? {
                        continue;
                    }
                    set_text(&mut wizard.card, field, value);
                    if field != Field::AppId {
                        wizard.note = None;
                    }
                    if !wizard.forward() {
                        tidy(&mut wizard.card);
                        return Ok(Some(wizard.card));
                    }
                }
                other => {
                    if wizard.input.edit(&other) {
                        wizard.error = None;
                        wizard.completion = None;
                    }
                }
            }
        }
    }

    /// Asks Discord whether the typed ID is a real application; `true` to go on.
    fn check_app(&mut self, wizard: &mut Wizard, value: &str) -> io::Result<bool> {
        let Ok(id) = config::parse_app_id(value) else {
            return Ok(false);
        };
        if wizard.unchecked == Some(id) {
            return Ok(true);
        }
        let job = std::thread::spawn(move || engine::application_name(id));
        while !job.is_finished() {
            let mut canvas = self.term.canvas();
            let cursor = self.draw_wizard(&mut canvas, wizard, true);
            self.term.present(&canvas, cursor)?;
            let _ = self.term.key(FRAME)?;
        }
        match job.join().unwrap_or_else(|_| Err("la vérification a échoué".to_owned())) {
            Ok(Some(name)) => {
                wizard.note = Some((Tone::Good, format!("Application trouvée : {name}")));
                Ok(true)
            }
            Ok(None) => {
                wizard.error = Some(
                    "Discord ne connaît pas cet ID : recopie l'Application ID de ton app".to_owned(),
                );
                Ok(false)
            }
            Err(problem) => {
                wizard.unchecked = Some(id);
                wizard.note = Some((
                    Tone::Warn,
                    format!("Vérification impossible ({problem}) : Entrée pour garder cet ID"),
                ));
                Ok(false)
            }
        }
    }

    /// Draws the question; returns where the text cursor goes.
    fn draw_wizard(&self, canvas: &mut Canvas, wizard: &Wizard, checking: bool) -> Option<(i32, i32)> {
        logo::header(canvas, self.theme, &format!("Configuration · carte {}", wizard.number));
        let field = wizard.field();
        let accent = self.theme.accent();
        let wide = canvas.w >= 104;
        let x = 4;
        let room = if wide { canvas.w - preview::WIDTH - 14 } else { canvas.w - 8 }.min(72);
        // Where we are: one dash per question, the done ones in the gradient.
        let shown = wizard.shown();
        let position = shown.iter().position(|index| *index == wizard.step).unwrap_or(0);
        canvas.text(
            x,
            4,
            &format!("Étape {} sur {}", position + 1, shown.len()),
            Style::fg(ink::FAINT),
        );
        let bar_x = x + 16;
        let bar = (room - 16).max(10);
        let done = ((position + 1) as f32 / shown.len() as f32 * bar as f32).round() as i32;
        for i in 0..bar {
            let colour = if i < done { self.theme.at(i as f32 / bar as f32) } else { ink::GHOST };
            canvas.put(bar_x + i, 4, '─', Style::fg(colour));
        }
        // The question slides in from the right as it appears.
        let k = (wizard.since.elapsed().as_secs_f32() / 0.3).min(1.0);
        let slide = ((1.0 - k) * (1.0 - k) * 6.0) as i32;
        let fade = |colour: super::paint::Rgb| colour.shade((1.0 - k) * 0.7);
        let mut title_end = canvas.text(x + slide, 6, field.title(), Style::fg(fade(ink::TEXT)).bold());
        if field.optional() && field.choices().is_none() {
            title_end = canvas.text(title_end + 2, 6, "facultatif", Style::fg(fade(ink::FAINT)));
        }
        let _ = title_end;
        let mut y = 7;
        for line in wrap(field.help(), room) {
            canvas.text(x + slide, y, &line, Style::fg(fade(ink::MUTED)));
            y += 1;
        }
        y += 1;
        let mut cursor = None;
        if let Some(options) = field.choices() {
            for (index, (label, hint)) in options.iter().enumerate() {
                let row = y + index as i32;
                let selected = index == wizard.choice;
                if selected {
                    canvas.fill(x - 1, row, room + 2, 1, accent.shade(0.8));
                    canvas.put(x - 1, row, '▌', Style::fg(accent));
                }
                canvas.text(x + 1, row, &(index + 1).to_string(), Style::fg(ink::FAINT));
                let style = if selected { Style::fg(ink::TEXT).bold() } else { Style::fg(ink::MUTED) };
                canvas.text(x + 4, row, label, style);
                canvas.text(
                    x + 24,
                    row,
                    &clip(hint, room - 25),
                    Style::fg(if selected { ink::MUTED } else { ink::FAINT }),
                );
            }
            y += options.len() as i32 + 1;
        } else {
            canvas.text(x, y, "›", Style::fg(accent).bold());
            let visible = (room - 4).max(8) as usize;
            let start = wizard.input.cursor.saturating_sub(visible - 1);
            let text: String = wizard.input.chars.iter().skip(start).take(visible).collect();
            if wizard.input.chars.is_empty() {
                let mut hint = field.placeholder().to_owned();
                if field == Field::Name
                    && let Some(app) = engine::known_application_name(wizard.card.app_id)
                {
                    hint = format!("vide = « {app} »");
                }
                canvas.text(x + 2, y, &clip(&hint, visible as i32), Style::fg(ink::GHOST));
            } else {
                canvas.text(x + 2, y, &text, Style::fg(ink::TEXT));
            }
            for i in 0..room - 2 {
                let colour = if wizard.error.is_some() {
                    ink::BAD.shade(0.4)
                } else {
                    self.theme.at(i as f32 / room as f32).shade(0.45)
                };
                canvas.put(x + 2 + i, y + 1, '─', Style::fg(colour));
            }
            let count = format!("{}/{}", wizard.input.chars.len(), field.max());
            canvas.text(x + room - width(&count), y + 2, &count, Style::fg(ink::FAINT));
            cursor = (!checking).then_some((x + 2 + (wizard.input.cursor - start) as i32, y));
            if field.picture() && wizard.card.app_id != 0 {
                self.draw_pictures(canvas, wizard, x, y + 2, room - width(&count) - 2);
            }
            y += 4;
        }
        if checking {
            let dots = ".".repeat(1 + (ticking() * 4.0) as usize % 3);
            canvas.put(x, y, '●', Style::fg(accent));
            canvas.text(x + 2, y, &format!("Vérification auprès de Discord{dots}"), Style::fg(ink::MUTED));
        } else if let Some(error) = &wizard.error {
            canvas.put(x, y, '●', Style::fg(ink::BAD));
            canvas.text(x + 2, y, &clip(error, room - 2), Style::fg(ink::BAD));
        } else if let Some((tone, note)) = &wizard.note {
            canvas.put(x, y, '●', Style::fg(tone.colour()));
            canvas.text(x + 2, y, &clip(note, room - 2), Style::fg(ink::TEXT));
        }
        if wide {
            let px = canvas.w - preview::WIDTH - 5;
            canvas.text(px, 6, "APERÇU", Style::fg(ink::FAINT).bold());
            preview::draw(canvas, px, 7, &wizard.view(), self.theme);
        }
        let hints: Vec<(&str, &str)> = if field.choices().is_some() {
            vec![("↑↓", "choisir"), ("Entrée", "valider"), ("Échap", "retour")]
        } else {
            let mut hints = vec![("Entrée", "valider"), ("Échap", "retour")];
            if field.picture() {
                hints.push(("Tab", "images de l'app"));
            }
            hints.extend([("Ctrl+V", "coller"), ("Ctrl+U", "effacer")]);
            hints
        };
        logo::footer(canvas, &hints);
        cursor
    }
}

impl App {
    /// The application's picture keys under the box: the ones matching what is typed, the one
    /// Tab put in the box lit.
    fn draw_pictures(&self, canvas: &mut Canvas, wizard: &Wizard, x: i32, y: i32, room: i32) {
        let Some(found) = wizard.pictures() else {
            canvas.text(x + 2, y, "Recherche des images de l'application…", Style::fg(ink::FAINT));
            return;
        };
        let all = engine::known_application_assets(wizard.card.app_id).unwrap_or_default();
        if all.is_empty() {
            canvas.text(
                x + 2,
                y,
                &clip("Pas d'image dans cette application : colle un lien https://", room - 2),
                Style::fg(ink::FAINT),
            );
            return;
        }
        if found.is_empty() {
            canvas.text(x + 2, y, "Aucune image ne correspond", Style::fg(ink::FAINT));
            return;
        }
        let mut column = canvas.text(x + 2, y, "Tab ", Style::fg(ink::TEXT).bold());
        let chosen = wizard.completion.as_ref().map(|(_, index)| *index);
        // Start the list a little before the chosen picture so it stays in view.
        let first = chosen.map_or(0, |index| index.saturating_sub(2));
        for (index, name) in found.iter().enumerate().skip(first) {
            if column + width(name) + 2 > x + room {
                canvas.text(column, y, "…", Style::fg(ink::FAINT));
                break;
            }
            let style = if Some(index) == chosen {
                Style::fg(self.theme.accent()).bold()
            } else {
                Style::fg(ink::MUTED)
            };
            column = canvas.text(column, y, name, style) + 2;
        }
    }
}

/// Seconds on a clock that only drives the "checking" dots.
fn ticking() -> f32 {
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}

/// Cuts a text into lines of at most `room` columns, at spaces.
fn wrap(text: &str, room: i32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && width(&line) + 1 + width(word) > room {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_fill_the_card() {
        let mut card = Activity::default();
        set_choice(&mut card, Field::Template, 2);
        assert_eq!(card.app_id, 880_218_394_199_220_334);
        assert_eq!(card.status, "watching");
        assert_eq!(card.buttons.len(), 1);
        // The questions that follow know the template's answers.
        assert_eq!(choice_of(&card, Field::Status), 3);
        assert!(Field::Link1.shown(&card, false));
        assert!(!Field::StreamUrl.shown(&card, false));
    }

    #[test]
    fn half_made_buttons_and_skipped_texts_are_tidied() {
        let mut card = Activity {
            details: "  hello  ".to_owned(),
            image_text: "orphan".to_owned(),
            stream_url: "https://twitch.tv/x".to_owned(),
            buttons: vec![
                Button {
                    label: "ok".to_owned(),
                    url: "https://ok.test".to_owned(),
                },
                Button {
                    label: "no link".to_owned(),
                    url: String::new(),
                },
            ],
            ..Activity::default()
        };
        tidy(&mut card);
        assert_eq!(card.details, "hello");
        assert!(card.image_text.is_empty());
        assert!(card.stream_url.is_empty());
        assert_eq!(card.buttons.len(), 1);
    }

    #[test]
    fn answers_are_checked_like_discord_does() {
        assert!(Field::AppId.check("abc").is_err());
        assert!(Field::Image.check("mon logo").is_err());
        assert!(Field::Image.check("logo").is_ok());
        assert!(Field::Image.check("ftp://x").is_err());
        assert!(Field::Link1.check("example.com").is_err());
        assert!(Field::Label1.check(&"x".repeat(33)).is_err());
        assert!(Field::Details.check("").is_ok());
    }

    #[test]
    fn text_wraps_at_spaces() {
        assert_eq!(wrap("un deux trois quatre", 9), ["un deux", "trois", "quatre"]);
    }

    #[test]
    fn the_input_edits_like_a_text_box() {
        let mut input = Input::default();
        input.paste("abc\n");
        input.edit(&Key::Left);
        input.edit(&Key::Backspace);
        input.edit(&Key::Char('X'));
        assert_eq!(input.value(), "aXc");
        input.edit(&Key::Ctrl('u'));
        assert_eq!(input.value(), "");
    }
}
