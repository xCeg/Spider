//! `spider.json`: the cards to publish and how to reach Discord.
//!
//! Every field is optional so a short file works; the default is one card, transport Auto,
//! refreshed every 15 s. `deny_unknown_fields` turns a typo into a clear error instead of a
//! setting that is silently ignored. Spider's window writes the same file (`save`), so a
//! hand-edited file and the wizard stay interchangeable.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::clock::{self, Local};
use crate::engine::{Client, Config, Mode, TimerMode};

/// The whole file.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// `false` clears the profile and waits; put it back to `true` to publish again.
    pub enabled: bool,
    /// `auto` (debug port first, then the pipe), `cdp` (debug port only) or `rpc`
    /// (local pipe only).
    pub transport: String,
    /// `auto`, `stable`, `ptb` or `canary`: the Discord client, and so the account, the cards
    /// go to when several run at once.
    pub client: String,
    /// Seconds between two refreshes of an unchanged card.
    pub refresh_seconds: f64,
    /// Seconds before retrying a card Discord refused.
    pub retry_seconds: f64,
    /// The colours of Spider's window (`venom`, `blood`, `ocean`, `toxic`, `sunset`, `ghost`).
    pub theme: String,
    /// Opening Spider starts the presence at once (no menu first).
    pub auto_start: bool,
    /// One to three cards, in slot order.
    pub activities: Vec<Activity>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            transport: "auto".to_owned(),
            client: "auto".to_owned(),
            refresh_seconds: 15.0,
            retry_seconds: 5.0,
            theme: "venom".to_owned(),
            auto_start: false,
            activities: Vec::new(),
        }
    }
}

/// One card. `spider.json` uses human words (`status: "watching"`, `platform: "ps5"`) and this
/// turns them into the numbers and `&'static str` the engine's `Config` wants.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Activity {
    /// 1 to 3. Left at 0, cards take 1, 2, 3 in file order.
    pub slot: u8,
    /// The Discord application the card is sent as (its name is the card's title).
    pub app_id: u64,
    /// The application whose pipe signs `https://` pictures; 0 uses `app_id`. Only matters
    /// when the picture is a URL: the CDP transport must send the signed `mp:external/...`
    /// path, and only a running Discord client can sign it.
    pub image_signer: u64,
    /// The title shown in the card; empty keeps the application's own name.
    pub name: String,
    /// The first line, under the title.
    pub details: String,
    /// The second line.
    pub state: String,
    /// An Art Asset key of the application (e.g. `logo`), or an `https://` picture.
    pub image: String,
    /// The big picture's hover text.
    pub image_text: String,
    /// The corner picture: an Art Asset key or an `https://` picture.
    pub small_image: String,
    /// The corner picture's hover text.
    pub small_text: String,
    /// `playing`, `streaming`, `listening`, `watching` or `competing`.
    pub status: String,
    /// Empty, `xbox`, `ps5` or `meta_quest`; the pipe always shows desktop.
    pub platform: String,
    /// A `streaming` card's Watch button; also the card's URL.
    pub stream_url: String,
    /// Up to two buttons (`label` + `https://` URL).
    pub buttons: Vec<Button>,
    /// Show a timer.
    pub timer: bool,
    /// What the timer shows: `start` (since the card appeared), `clock` (the time of day) or
    /// `countdown` (`countdown_minutes` going down, then again).
    pub timer_mode: String,
    pub countdown_minutes: u32,
    /// `[2, 4]` shows "(2 of 4)" after the second line; empty = none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub party: Vec<u32>,
    /// Other first lines, shown in turn with `details`, each for `rotation_seconds`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rotation: Vec<String>,
    pub rotation_seconds: u32,
}

impl Default for Activity {
    fn default() -> Self {
        Self {
            slot: 0,
            app_id: 0,
            image_signer: 0,
            name: String::new(),
            details: String::new(),
            state: String::new(),
            image: String::new(),
            image_text: String::new(),
            small_image: String::new(),
            small_text: String::new(),
            status: "playing".to_owned(),
            platform: String::new(),
            stream_url: String::new(),
            buttons: Vec::new(),
            timer: true,
            timer_mode: "start".to_owned(),
            countdown_minutes: 30,
            party: Vec::new(),
            rotation: Vec::new(),
            rotation_seconds: 30,
        }
    }
}

/// One link button.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Button {
    pub label: String,
    pub url: String,
}

/// `spider.json` beside the executable: a double-click, a shortcut and the background build all
/// find the same file.
pub fn default_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|folder| folder.join("spider.json")))
        .unwrap_or_else(|| std::path::PathBuf::from("spider.json"))
}

/// The activity types, as `spider.json` spells them, in the order the window offers them.
pub const STATUSES: [&str; 5] = ["playing", "streaming", "listening", "watching", "competing"];

/// The console badges (empty = none), in the order the window offers them.
pub const PLATFORMS: [&str; 4] = ["", "xbox", "ps5", "meta_quest"];

/// The transports, in the order the window offers them.
pub const TRANSPORTS: [&str; 3] = ["auto", "cdp", "rpc"];

/// The timer modes, in the order the window offers them.
pub const TIMER_MODES: [&str; 3] = ["start", "clock", "countdown"];

/// Discord updates a card at most every 15 seconds: a faster rotation would be dropped.
pub const ROTATION_MIN: u32 = 15;

/// The words a text may hold, replaced when the card is sent: `{heure}`, `{date}`, `{jour}`.
pub fn expand(text: &str, now: &Local) -> String {
    if !text.contains('{') {
        return text.to_owned();
    }
    text.replace("{heure}", &now.hour_minute())
        .replace("{date}", &now.date())
        .replace("{jour}", now.weekday_name())
}

impl Activity {
    /// The first line shown at `epoch_seconds`: `details` and the `rotation` lines in turn.
    pub fn details_at(&self, epoch_seconds: u64) -> &str {
        let lines: Vec<&str> = std::iter::once(self.details.as_str())
            .chain(self.rotation.iter().map(String::as_str))
            .filter(|line| !line.trim().is_empty())
            .collect();
        if lines.len() < 2 {
            return &self.details;
        }
        let every = u64::from(self.rotation_seconds.max(ROTATION_MIN));
        lines[(epoch_seconds / every % lines.len() as u64) as usize]
    }

    /// The timer, as the engine counts it.
    pub fn timer_mode(&self) -> TimerMode {
        match self.timer_mode.trim().to_ascii_lowercase().as_str() {
            "clock" | "heure" => TimerMode::Clock,
            "countdown" | "rebours" => TimerMode::Countdown(self.countdown_minutes.clamp(1, 1440)),
            _ => TimerMode::Start,
        }
    }

    /// The group size, when it makes sense: at least one, and no more than the maximum.
    pub fn party_size(&self) -> Option<(u32, u32)> {
        match self.party[..] {
            [size, max] if size >= 1 && max >= size => Some((size, max)),
            _ => None,
        }
    }
}

impl Settings {
    /// Reads and checks the file.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("{} : {error}", path.display()))?;
        // Notepad saves UTF-8 with a byte order mark, which JSON does not allow.
        let text = text.trim_start_matches('\u{feff}');
        let settings: Settings =
            serde_json::from_str(text).map_err(|error| format!("{} : {error}", path.display()))?;
        // Check the cards now, so a broken file is reported once instead of on every write.
        settings.activities()?;
        Ok(settings)
    }

    /// Writes the file, through a temporary one so a watcher never reads half of it.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let mut text = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        text.push('\n');
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, text)
            .map_err(|error| format!("{} : {error}", temporary.display()))?;
        std::fs::rename(&temporary, path).map_err(|error| format!("{} : {error}", path.display()))
    }

    pub fn mode(&self) -> Mode {
        match self.transport.trim().to_ascii_lowercase().as_str() {
            "cdp" => Mode::Cdp,
            "rpc" => Mode::Rpc,
            _ => Mode::Auto,
        }
    }

    pub fn client(&self) -> Client {
        Client::parse(&self.client)
    }

    pub fn refresh(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(self.refresh_seconds.max(2.0))
    }

    pub fn retry(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(self.retry_seconds.max(1.0))
    }

    /// The cards, ready for the engine, as they read now. Fails on a file that would publish
    /// nothing useful.
    pub fn activities(&self) -> Result<Vec<Config>, String> {
        self.activities_at(&clock::now(), clock::epoch_ms() / 1000)
    }

    /// The cards as they read at a given moment (rotation, `{heure}`, `{date}`, `{jour}`).
    pub fn activities_at(&self, now: &Local, epoch_seconds: u64) -> Result<Vec<Config>, String> {
        if self.activities.is_empty() {
            return Err("aucune carte : remplis la liste \"activities\"".to_owned());
        }
        if self.activities.len() > 3 {
            return Err("Discord garde trois activités au maximum".to_owned());
        }
        let mut seen = Vec::new();
        let mut cards = Vec::with_capacity(self.activities.len());
        for (index, activity) in self.activities.iter().enumerate() {
            if activity.app_id == 0 {
                return Err(format!("carte {} : app_id est obligatoire", index + 1));
            }
            let slot = if activity.slot == 0 {
                index as u8 + 1
            } else {
                activity.slot
            };
            if !(1..=3).contains(&slot) {
                return Err(format!("carte {} : le slot va de 1 à 3", index + 1));
            }
            if seen.contains(&slot) {
                return Err(format!("carte {} : le slot {slot} est pris deux fois", index + 1));
            }
            seen.push(slot);
            let console = platform(&activity.platform);
            let (small_image, small_text) = corner(activity, console);
            cards.push(Config {
                app_id: activity.app_id,
                slot,
                image_signer: activity.image_signer,
                name: expand(&activity.name, now),
                details: expand(activity.details_at(epoch_seconds), now),
                state: expand(&activity.state, now),
                image: activity.image.clone(),
                image_text: expand(&activity.image_text, now),
                small_image,
                small_text: expand(&small_text, now),
                kind: kind(&activity.status),
                platform: console,
                stream_url: activity.stream_url.clone(),
                buttons: buttons(activity),
                timer: activity.timer,
                timer_mode: activity.timer_mode(),
                party: activity.party_size(),
            });
        }
        Ok(cards)
    }

    /// What will not show the way the file asks, in plain words (the file is still valid).
    pub fn warnings(&self) -> Vec<String> {
        let mut list = Vec::new();
        let mode = self.mode();
        for (index, activity) in self.activities.iter().enumerate() {
            let number = index + 1;
            if mode == Mode::Rpc && kind(&activity.status) == 1 {
                list.push(format!(
                    "carte {number} : le pipe ne connaît pas « Streame », elle s'affichera en « Joue à »"
                ));
            }
            if mode == Mode::Rpc && platform(&activity.platform).is_some() {
                list.push(format!(
                    "carte {number} : par le pipe, la console n'apparaît qu'en pastille sur l'image"
                ));
            }
            if !activity.party.is_empty() && activity.party_size().is_none() {
                list.push(format!(
                    "carte {number} : le groupe s'écrit [taille, maximum], par exemple [2, 4]"
                ));
            }
            if !activity.rotation.is_empty() && activity.rotation_seconds < ROTATION_MIN {
                list.push(format!(
                    "carte {number} : Discord ne change une carte que toutes les 15 secondes"
                ));
            }
            let dropped = activity
                .buttons
                .iter()
                .filter(|button| !button.label.trim().is_empty() && !is_url(button.url.trim()))
                .count();
            if dropped > 0 {
                list.push(format!(
                    "carte {number} : un bouton sans lien http(s):// est ignoré"
                ));
            }
        }
        if mode != Mode::Cdp {
            // One pipe connection per application: a second card of the same application
            // replaces the first one there.
            for (index, activity) in self.activities.iter().enumerate() {
                if let Some(first) = self.activities[..index]
                    .iter()
                    .position(|earlier| earlier.app_id == activity.app_id && activity.app_id != 0)
                {
                    list.push(format!(
                        "cartes {} et {} : même application ; par le pipe, seule la dernière reste",
                        first + 1,
                        index + 1
                    ));
                }
            }
        }
        list
    }
}

/// The activity type, in Vencord's wording. Unknown words mean Playing.
pub fn kind(status: &str) -> u8 {
    match status.trim().to_ascii_lowercase().as_str() {
        "streaming" => 1,
        "listening" => 2,
        "watching" => 3,
        "competing" => 5,
        _ => 0,
    }
}

/// The console glyph. An unknown word means no platform field.
pub fn platform(platform: &str) -> Option<&'static str> {
    match platform.trim().to_ascii_lowercase().as_str() {
        "xbox" => Some("xbox"),
        "ps5" | "playstation" => Some("ps5"),
        "meta_quest" | "quest" | "vr" => Some("meta_quest"),
        _ => None,
    }
}

/// The console badges Cinnabug used: a picture in the card's corner, which every transport
/// shows (the pipe drops the `platform` field itself, so this is all a pipe card gets).
pub const CONSOLE_BADGES: [(&str, &str, &str); 3] = [
    ("xbox", "https://files.catbox.moe/8uy93z.png", "Xbox"),
    ("ps5", "https://files.catbox.moe/q3mrvb.png", "PlayStation"),
    ("meta_quest", "https://files.catbox.moe/29yy6c.png", "VR"),
];

/// The corner picture and its text: the card's own, else the console's badge.
fn corner(activity: &Activity, console: Option<&'static str>) -> (String, String) {
    let own = activity.small_image.trim();
    if !own.is_empty() {
        return (own.to_owned(), activity.small_text.clone());
    }
    match console.and_then(|console| CONSOLE_BADGES.iter().find(|(word, _, _)| *word == console)) {
        Some((_, picture, name)) => {
            let text = if activity.small_text.trim().is_empty() {
                (*name).to_owned()
            } else {
                activity.small_text.clone()
            };
            ((*picture).to_owned(), text)
        }
        None => (String::new(), activity.small_text.clone()),
    }
}

/// Discord shows two buttons at most, and refuses anything that is not a link.
fn buttons(activity: &Activity) -> Vec<(String, String)> {
    activity
        .buttons
        .iter()
        .filter_map(|button| {
            let (label, url) = (button.label.trim(), button.url.trim());
            (!label.is_empty() && is_url(url)).then(|| (label.to_owned(), url.to_owned()))
        })
        .take(2)
        .collect()
}

pub fn is_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// An application ID as typed: digits only (spaces ignored), the length of a Discord ID.
pub fn parse_app_id(text: &str) -> Result<u64, String> {
    let digits: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.is_empty() {
        return Err("l'ID est obligatoire".to_owned());
    }
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err("un ID ne contient que des chiffres".to_owned());
    }
    if !(17..=20).contains(&digits.len()) {
        return Err("un ID Discord fait 17 à 20 chiffres".to_owned());
    }
    match digits.parse::<u64>() {
        Ok(0) | Err(_) => Err("cet ID n'est pas valide".to_owned()),
        Ok(id) => Ok(id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Settings, String> {
        let settings: Settings = serde_json::from_str(text).map_err(|error| error.to_string())?;
        settings.activities()?;
        Ok(settings)
    }

    #[test]
    fn a_short_file_is_one_playing_card_with_a_timer() {
        let settings = parse(r#"{"activities":[{"app_id":123,"details":"hi"}]}"#).unwrap();
        let cards = settings.activities().unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].slot, 1);
        assert_eq!(cards[0].kind, 0);
        assert!(cards[0].timer);
        assert_eq!(settings.mode(), Mode::Auto);
        assert_eq!(settings.client(), Client::Auto);
    }

    #[test]
    fn slots_are_filled_in_order_and_can_be_given() {
        let cards = parse(
            r#"{"activities":[{"app_id":1},{"app_id":2,"slot":3},{"app_id":3,"slot":2}]}"#,
        )
        .unwrap()
        .activities()
        .unwrap();
        assert_eq!(
            cards.iter().map(|card| card.slot).collect::<Vec<_>>(),
            [1, 3, 2]
        );
    }

    #[test]
    fn words_become_types_platforms_and_buttons() {
        let cards = parse(
            r#"{"activities":[{"app_id":1,"status":"Watching","platform":"ps5",
                "buttons":[{"label":"Watch","url":"https://twitch.tv/x"},
                           {"label":"bad","url":"javascript:1"},
                           {"label":"two","url":"https://b"},
                           {"label":"three","url":"https://c"}]}]}"#,
        )
        .unwrap()
        .activities()
        .unwrap();
        assert_eq!(cards[0].kind, 3);
        assert_eq!(cards[0].platform, Some("ps5"));
        assert_eq!(cards[0].buttons.len(), 2);
        assert_eq!(cards[0].buttons[0].0, "Watch");
    }

    #[test]
    fn a_broken_file_is_reported() {
        assert!(parse(r#"{"activities":[]}"#).is_err());
        assert!(parse(r#"{"activities":[{"app_id":0}]}"#).is_err());
        assert!(parse(r#"{"activities":[{"app_id":1,"slot":2},{"app_id":2,"slot":2}]}"#).is_err());
        assert!(parse(r#"{"activitees":[{"app_id":1}]}"#).is_err());
    }

    #[test]
    fn refresh_and_retry_are_clamped() {
        let settings =
            parse(r#"{"refresh_seconds":0,"retry_seconds":-4,"activities":[{"app_id":1}]}"#)
                .unwrap();
        assert_eq!(settings.refresh().as_secs_f64(), 2.0);
        assert_eq!(settings.retry().as_secs_f64(), 1.0);
    }

    #[test]
    fn a_saved_file_reads_back_the_same() {
        let path = std::env::temp_dir().join(format!("spider-test-{}.json", std::process::id()));
        let mut settings = Settings {
            client: "ptb".to_owned(),
            theme: "ocean".to_owned(),
            ..Settings::default()
        };
        settings.activities.push(Activity {
            app_id: 123_456_789_012_345_678,
            details: "Une ligne avec des accents : é à ç".to_owned(),
            buttons: vec![Button {
                label: "Site".to_owned(),
                url: "https://example.test/".to_owned(),
            }],
            ..Activity::default()
        });
        settings.save(&path).unwrap();
        let read = Settings::load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(read, settings);
        assert_eq!(read.client(), Client::Ptb);
    }

    #[test]
    fn a_file_saved_with_a_byte_order_mark_still_loads() {
        let path = std::env::temp_dir().join(format!("spider-bom-{}.json", std::process::id()));
        std::fs::write(&path, "\u{feff}{\"activities\":[{\"app_id\":1}]}").unwrap();
        let read = Settings::load(&path);
        let _ = std::fs::remove_file(&path);
        assert!(read.is_ok(), "{read:?}");
    }

    #[test]
    fn warnings_explain_what_the_pipe_cannot_show() {
        let settings = parse(
            r#"{"transport":"rpc","activities":[
                {"app_id":1,"status":"streaming","platform":"xbox"},
                {"app_id":1,"slot":2}]}"#,
        )
        .unwrap();
        let warnings = settings.warnings();
        assert_eq!(warnings.len(), 3, "{warnings:?}");
    }

    #[test]
    fn a_console_brings_its_badge_unless_the_card_has_a_corner_picture() {
        let cards = parse(
            r#"{"activities":[{"app_id":1,"platform":"xbox"},
                {"app_id":2,"platform":"ps5","small_image":"mine","small_text":"Mine"}]}"#,
        )
        .unwrap()
        .activities()
        .unwrap();
        assert_eq!(cards[0].small_image, CONSOLE_BADGES[0].1);
        assert_eq!(cards[0].small_text, "Xbox");
        assert_eq!(cards[1].small_image, "mine");
        assert_eq!(cards[1].small_text, "Mine");
    }

    #[test]
    fn rotation_placeholders_timer_modes_and_groups_reach_the_cards() {
        let settings = parse(
            r#"{"activities":[{"app_id":1,"details":"Je code ({heure})","state":"On est {jour} {date}",
                "rotation":["Je teste","",
                "Je dors"],"rotation_seconds":30,
                "timer_mode":"countdown","countdown_minutes":5,"party":[2,4]}]}"#,
        )
        .unwrap();
        let now = Local {
            year: 2026,
            month: 11,
            day: 16,
            weekday: 1,
            hour: 21,
            minute: 7,
            second: 0,
            millis: 0,
        };
        let at = |seconds: u64| settings.activities_at(&now, seconds).unwrap().remove(0);
        // Three non-empty lines, 30 s each, in turn.
        assert_eq!(at(0).details, "Je code (21:07)");
        assert_eq!(at(30).details, "Je teste");
        assert_eq!(at(65).details, "Je dors");
        assert_eq!(at(90).details, "Je code (21:07)");
        let card = at(0);
        assert_eq!(card.state, "On est lundi 16/11/2026");
        assert_eq!(card.timer_mode, TimerMode::Countdown(5));
        assert_eq!(card.party, Some((2, 4)));
        // A group bigger than its maximum is left out, and said.
        let wrong = parse(r#"{"activities":[{"app_id":1,"party":[5,4]}]}"#).unwrap();
        assert_eq!(wrong.activities().unwrap()[0].party, None);
        assert_eq!(wrong.warnings().len(), 1);
    }

    #[test]
    fn app_ids_are_checked_like_discord_ids() {
        assert_eq!(parse_app_id("1234 5678 9012 3456 78"), Ok(123_456_789_012_345_678));
        assert!(parse_app_id("").is_err());
        assert!(parse_app_id("12ab").is_err());
        assert!(parse_app_id("1234").is_err());
        assert!(parse_app_id("000000000000000000").is_err());
    }
}
