//! Saved configurations: `profiles/<name>.json` beside `spider.json`, in the same format.
//! Loading one replaces the cards and their settings; the window's own preferences (theme,
//! start-up) stay as they are.

use std::path::{Path, PathBuf};

use crate::config::Settings;

/// The profiles' folder, beside the configuration file.
pub fn folder(config: &Path) -> PathBuf {
    config
        .parent()
        .map_or_else(|| PathBuf::from("profiles"), |parent| parent.join("profiles"))
}

/// A name that is safe as a file name: letters, digits, spaces, `-` and `_`, 40 characters
/// at most; `None` when nothing usable is left.
pub fn clean_name(name: &str) -> Option<String> {
    let kept: String = name
        .trim()
        .chars()
        .filter(|ch| ch.is_alphanumeric() || matches!(ch, ' ' | '-' | '_'))
        .take(40)
        .collect();
    let kept = kept.trim().to_owned();
    (!kept.is_empty()).then_some(kept)
}

/// Every profile, by name, with its content (or why it cannot be read).
pub fn list(config: &Path) -> Vec<(String, Result<Settings, String>)> {
    let mut found: Vec<(String, Result<Settings, String>)> = std::fs::read_dir(folder(config))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|extension| extension == "json"))
                .filter_map(|path| {
                    let name = path.file_stem()?.to_string_lossy().into_owned();
                    Some((name, Settings::load(&path)))
                })
                .collect()
        })
        .unwrap_or_default();
    found.sort_by_key(|(name, _)| name.to_lowercase());
    found
}

/// Saves `settings` as the profile `name` (replacing one of the same name).
pub fn save(config: &Path, name: &str, settings: &Settings) -> Result<String, String> {
    let name = clean_name(name).ok_or("ce nom ne peut pas servir de nom de fichier")?;
    let folder = folder(config);
    std::fs::create_dir_all(&folder).map_err(|error| format!("{} : {error}", folder.display()))?;
    settings.save(&folder.join(format!("{name}.json")))?;
    Ok(name)
}

/// The profile `name`, ready to become the configuration: its cards and their settings, with
/// `current`'s window preferences.
pub fn load(config: &Path, name: &str, current: Option<&Settings>) -> Result<Settings, String> {
    let mut profile = Settings::load(&folder(config).join(format!("{name}.json")))?;
    if let Some(current) = current {
        profile.theme = current.theme.clone();
        profile.auto_start = current.auto_start;
    }
    Ok(profile)
}

/// Deletes the profile `name`.
pub fn delete(config: &Path, name: &str) -> Result<(), String> {
    let path = folder(config).join(format!("{name}.json"));
    std::fs::remove_file(&path).map_err(|error| format!("{} : {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Activity;

    #[test]
    fn profiles_are_saved_listed_loaded_and_deleted() {
        let root = std::env::temp_dir().join(format!("spider-profiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let config = root.join("spider.json");
        let mut gaming = Settings::default();
        gaming.activities.push(Activity {
            app_id: 123_456_789_012_345_678,
            details: "En ranked".to_owned(),
            ..Activity::default()
        });
        gaming.theme = "blood".to_owned();
        // Characters a file name cannot hold are dropped (no "../" escaping the folder).
        assert_eq!(save(&config, "  Gaming: ranked/../ ", &gaming).unwrap(), "Gaming ranked");
        assert!(save(&config, "../..", &gaming).is_err());
        let found = list(&config);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "Gaming ranked");
        // Loading keeps the window's own theme.
        let current = Settings {
            theme: "ocean".to_owned(),
            ..Settings::default()
        };
        let loaded = load(&config, "Gaming ranked", Some(&current)).unwrap();
        assert_eq!(loaded.activities[0].details, "En ranked");
        assert_eq!(loaded.theme, "ocean");
        delete(&config, "Gaming ranked").unwrap();
        assert!(list(&config).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
