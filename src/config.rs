//! `spider.json`: the cards to publish and how to reach Discord.
//!
//! Every field is optional so a short file works; the default is one card, transport Auto,
//! refreshed every 15 s. `deny_unknown_fields` turns a typo into a clear error instead of a
//! setting that is silently ignored. Spider's window writes the same file (`save`), so a
//! hand-edited file and the wizard stay interchangeable.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::engine::{Client, Config, Mode};

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
    /// Show the elapsed time.
    pub timer: bool,
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

/// The activity types, as `spider.json` spells them, in the order the window offers them.
pub const STATUSES: [&str; 5] = ["playing", "streaming", "listening", "watching", "competing"];

/// The console badges (empty = none), in the order the window offers them.
pub const PLATFORMS: [&str; 4] = ["", "xbox", "ps5", "meta_quest"];

/// The transports, in the order the window offers them.
pub const TRANSPORTS: [&str; 3] = ["auto", "cdp", "rpc"];

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

    /// The cards, ready for the engine. Fails on a file that would publish nothing useful.
    pub fn activities(&self) -> Result<Vec<Config>, String> {
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
            cards.push(Config {
                app_id: activity.app_id,
                slot,
                image_signer: activity.image_signer,
                name: activity.name.clone(),
                details: activity.details.clone(),
                state: activity.state.clone(),
                image: activity.image.clone(),
                image_text: activity.image_text.clone(),
                small_image: activity.small_image.clone(),
                small_text: activity.small_text.clone(),
                kind: kind(&activity.status),
                platform: platform(&activity.platform),
                stream_url: activity.stream_url.clone(),
                buttons: buttons(activity),
                timer: activity.timer,
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
                    "carte {number} : le badge console ne passe que par le port de debug"
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
    fn app_ids_are_checked_like_discord_ids() {
        assert_eq!(parse_app_id("1234 5678 9012 3456 78"), Ok(123_456_789_012_345_678));
        assert!(parse_app_id("").is_err());
        assert!(parse_app_id("12ab").is_err());
        assert!(parse_app_id("1234").is_err());
        assert!(parse_app_id("000000000000000000").is_err());
    }
}
