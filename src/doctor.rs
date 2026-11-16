//! The diagnostic: what is running, what answers, and what the file asks for.
//!
//! Nothing here publishes a card. Discord's pipes get the same short handshake a game makes
//! when it starts, and applications are looked up on Discord's public API (no token).

use std::path::Path;

use crate::config::Settings;
use crate::engine::{self, Client};

/// How a line reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Level {
    Good,
    Warn,
    Bad,
    /// Not running, not needed: neither good nor bad.
    Off,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub level: Level,
    pub label: String,
    pub detail: String,
}

#[derive(Clone, Debug)]
pub struct Section {
    pub title: &'static str,
    pub lines: Vec<Line>,
}

fn line(level: Level, label: impl Into<String>, detail: impl Into<String>) -> Line {
    Line {
        level,
        label: label.into(),
        detail: detail.into(),
    }
}

/// Applications the diagnostic may shake hands as: Discord limits each application to about
/// two quick connections, so the check uses one the cards do not, and leaves their budget alone.
const NEUTRAL_APPS: [u64; 3] = [
    engine::SPOTIFY_APP_ID,
    401_518_684_763_586_560,
    981_509_069_309_354_054,
];

/// Runs every check (a second or two: it asks Discord and the Internet).
///
/// Each Discord process is asked for its account once per launch (the answer is kept); the
/// presence's own connection is never touched, and nothing is published.
pub fn check(path: &Path) -> Vec<Section> {
    let settings = Settings::load(path);
    let used: Vec<u64> = settings
        .as_ref()
        .map(|settings| settings.activities.iter().map(|activity| activity.app_id).collect())
        .unwrap_or_default();
    let neutral = NEUTRAL_APPS
        .into_iter()
        .find(|id| !used.contains(id))
        .unwrap_or(engine::SPOTIFY_APP_ID);
    let chosen = settings
        .as_ref()
        .map_or(Client::Auto, Settings::client);
    let mut sections = Vec::new();

    // Discord clients and their accounts.
    let accounts = engine::accounts(neutral);
    let found = accounts.clone().unwrap_or_else(|_| engine::clients());
    let mut lines = Vec::new();
    let local = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from);
    let mut installed_any = false;
    for client in [Client::Stable, Client::Ptb, Client::Canary] {
        let Some((folder, exe)) = client.install() else {
            continue;
        };
        let installed = local
            .as_ref()
            .is_some_and(|local| local.join(folder).join(exe).is_file());
        installed_any |= installed;
        let running = engine::discord_running(exe);
        let pipe = found.iter().find(|info| info.client == client);
        let chosen_mark = if chosen == client { "  (choisi)" } else { "" };
        match (running, pipe) {
            (_, Some(info)) => lines.push(line(
                Level::Good,
                client.label(),
                match &info.user {
                    Some(user) => format!("connecté : {user}  ·  discord-ipc-{}{chosen_mark}", info.pipe),
                    None => format!("ouvert  ·  discord-ipc-{}{chosen_mark}", info.pipe),
                },
            )),
            (true, None) => lines.push(line(
                Level::Warn,
                client.label(),
                "ouvert, pas encore de pipe (il démarre ?)",
            )),
            (false, None) if installed => lines.push(line(
                if chosen == client {
                    Level::Bad
                } else {
                    Level::Off
                },
                client.label(),
                if chosen == client {
                    "fermé : c'est le client choisi, ouvre-le".to_owned()
                } else {
                    "fermé".to_owned()
                },
            )),
            _ => {}
        }
    }
    if !installed_any && lines.is_empty() {
        lines.push(line(
            Level::Bad,
            "Discord",
            "introuvable : installe Discord (ou ouvre-le)",
        ));
    }
    // Whether Discord answers a connection at all (a handshake shows nothing on the profile).
    if !found.is_empty() {
        lines.push(match &accounts {
            Ok(list) if list.iter().any(|info| info.user.is_some()) => line(
                Level::Good,
                "Liaison",
                "Discord répond aux connexions (rien n'est publié pour le vérifier)",
            ),
            Ok(_) => line(Level::Warn, "Liaison", "pas de réponse de Discord pour l'instant"),
            Err(error) => line(Level::Bad, "Liaison", error.clone()),
        });
    }
    sections.push(Section {
        title: "Discord",
        lines,
    });

    // The debug port.
    let mut lines = Vec::new();
    match engine::debug_port_client() {
        Some(client) => {
            lines.push(line(
                Level::Good,
                "Port de debug",
                format!("ouvert par {} (127.0.0.1:{})", client.label(), engine::debug_port()),
            ));
            lines.push(line(
                Level::Warn,
                "Prudence",
                "tant qu'il est ouvert, un programme de ce PC peut piloter Discord",
            ));
        }
        None => lines.push(line(
            Level::Off,
            "Port de debug",
            "fermé : le pipe suffit (Streaming et badges console demandent « Relancer Discord »)",
        )),
    }
    sections.push(Section {
        title: "Port de debug",
        lines,
    });

    // The file.
    let mut lines = Vec::new();
    let name = path
        .file_name()
        .map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
    match &settings {
        Ok(settings) => {
            let count = settings.activities.len();
            lines.push(line(
                Level::Good,
                name,
                format!(
                    "{count} carte{}  ·  transport {}  ·  client {}",
                    if count > 1 { "s" } else { "" },
                    settings.transport,
                    settings.client().label()
                ),
            ));
            if !settings.enabled {
                lines.push(line(
                    Level::Warn,
                    "Désactivé",
                    "« enabled » est à false : aucune carte ne sera publiée",
                ));
            }
            for warning in settings.warnings() {
                lines.push(line(Level::Warn, "Attention", warning));
            }
        }
        Err(_) if !path.exists() => lines.push(line(
            Level::Warn,
            name,
            "pas encore de configuration : lance Spider et choisis « Configurer »",
        )),
        Err(error) => lines.push(line(Level::Bad, name, error.clone())),
    }
    sections.push(Section {
        title: "Configuration",
        lines,
    });

    // The applications the cards wear.
    if let Ok(settings) = &settings {
        let mut lines = Vec::new();
        for (index, activity) in settings.activities.iter().enumerate() {
            let label = format!("Carte {}", index + 1);
            lines.push(match engine::application_name(activity.app_id) {
                Ok(Some(app)) => line(Level::Good, label, format!("« {app} »  ·  {}", activity.app_id)),
                Ok(None) => line(
                    Level::Bad,
                    label,
                    format!("{} : Discord ne connaît pas cet ID", activity.app_id),
                ),
                Err(error) => line(Level::Warn, label, format!("vérification impossible : {error}")),
            });
        }
        sections.push(Section {
            title: "Applications",
            lines,
        });
    }
    sections
}
