//! `spider`: Discord Rich Presence from a file, with a console window to set it up.
//!
//! Without arguments in a console, Spider opens its window (animated logo, menu, wizard,
//! dashboard). The commands keep working for scripts and shortcuts: `spider run` keeps the
//! cards up without the window, `spider doctor` prints the diagnostic, and so on.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use spider::config::{Activity, Settings};
use spider::doctor::{self, Level};
use spider::engine::{self, Transport};
use spider::runner::{self, Log, Runner};
use spider::ui::{logo, paint};

fn main() {
    std::process::exit(cli());
}

fn cli() -> i32 {
    let mut command: Option<String> = None;
    let mut config: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" | "help" => {
                help();
                return 0;
            }
            "-V" | "--version" => {
                println!("spider {}", env!("CARGO_PKG_VERSION"));
                return 0;
            }
            "-c" | "--config" => match args.next() {
                Some(path) => config = Some(PathBuf::from(path)),
                None => {
                    eprintln!("--config attend un chemin");
                    return 2;
                }
            },
            other if other.starts_with('-') => {
                eprintln!("option inconnue : {other}");
                help();
                return 2;
            }
            other => command = Some(other.to_owned()),
        }
    }
    let path = config.unwrap_or_else(spider::config::default_path);
    runner::clear_on_close();
    match command.as_deref() {
        None if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() => {
            match spider::ui::run(path.clone()) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("la fenêtre n'a pas pu s'ouvrir ({error}) : mode texte");
                    run_loop(&path)
                }
            }
        }
        None | Some("run") => run_loop(&path),
        Some("once") => once(&path),
        Some("clear") => clear(),
        Some("init") => init(&path),
        Some("doctor") => doctor_text(&path),
        Some("restart") => restart(&path),
        Some(other) => {
            eprintln!("commande inconnue : {other}");
            help();
            2
        }
    }
}

/// Whether colour codes reach a console that understands them.
fn colours() -> bool {
    if !std::io::stdout().is_terminal() {
        return false;
    }
    #[cfg(windows)]
    {
        crossterm::ansi_support::supports_ansi()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// The logo, still, at the top of the text commands.
fn banner(path: &Path) {
    if !colours() {
        println!("SPIDER · {}\n", logo::subtitle());
        return;
    }
    let theme = Settings::load(path)
        .ok()
        .map_or(&paint::THEMES[0], |settings| paint::theme(&settings.theme));
    let mut canvas = paint::Canvas::new(logo::WIDTH + 8, logo::HERO_HEIGHT + 2);
    logo::hero(&mut canvas, 1, theme, logo::INTRO, None);
    for y in 0..canvas.h {
        println!("{}", canvas.row(y));
    }
}

fn help() {
    println!(
        "\
spider {} — Discord Rich Presence, sans token

usage :
  spider                      ouvre la fenêtre (menu, assistant, tableau de bord)
  spider run                  publie et suit spider.json, sans fenêtre
  spider once                 publie une fois puis quitte
  spider clear                retire les cartes (pipe et port de debug)
  spider doctor               diagnostic : Discord, comptes, port de debug, applications
  spider init                 écrit un spider.json d'exemple
  spider restart              relance Discord avec le port de debug (le ferme d'abord)

options :
  -c, --config CHEMIN   le fichier à lire (défaut : spider.json à côté de spider.exe)
  -h, --help            ce texte
  -V, --version         la version

Le journal s'écrit à côté du fichier (spider.log).",
        env!("CARGO_PKG_VERSION")
    );
}

fn run_loop(path: &Path) -> i32 {
    banner(path);
    let mut log = Log::new(path, true);
    if !path.is_file() {
        log.line(&format!(
            "{} introuvable : lance « spider » sans argument pour le configurer",
            path.display()
        ));
        return 1;
    }
    println!("  Ctrl+C pour arrêter : les cartes sont retirées en partant.\n");
    let mut runner = Runner::new(path);
    loop {
        let wait = runner.step(&mut |line| log.line(line));
        std::thread::sleep(wait);
    }
}

fn once(path: &Path) -> i32 {
    let settings = match Settings::load(path) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let cards = match settings.activities() {
        Ok(cards) => cards,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    engine::set_client(settings.client());
    match engine::publish(&cards, settings.mode(), None, false) {
        Ok(used) => {
            let plural = if cards.len() > 1 { "s" } else { "" };
            println!("{} : {} carte{plural} en ligne", used.label(), cards.len());
            if used == Transport::Rpc {
                println!("par le pipe, la carte part avec Spider : « spider run » la garde en ligne");
            }
            0
        }
        Err(error) => {
            eprintln!("échec : {error} | {}", engine::describe());
            1
        }
    }
}

fn clear() -> i32 {
    match engine::clear_all() {
        Ok(()) => {
            println!("cartes retirées");
            0
        }
        Err(error) => {
            eprintln!("échec : {error} | {}", engine::describe());
            1
        }
    }
}

fn init(path: &Path) -> i32 {
    if path.exists() {
        println!("{} existe déjà", path.display());
        return 0;
    }
    let mut settings = Settings::default();
    settings.activities.push(Activity {
        details: "Hello from Spider".to_owned(),
        ..Activity::default()
    });
    match settings.save(path) {
        Ok(()) => {
            println!(
                "{} écrit : mets ton app_id (ou lance « spider » pour l'assistant)",
                path.display()
            );
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn doctor_text(path: &Path) -> i32 {
    banner(path);
    let colour = colours();
    let mut worst = 0;
    for section in doctor::check(path) {
        println!("  {}", section.title.to_uppercase());
        for line in section.lines {
            let (rgb, word) = match line.level {
                Level::Good => ((87, 242, 135), "ok"),
                Level::Warn => ((254, 200, 64), "!!"),
                Level::Bad => ((255, 92, 102), "XX"),
                Level::Off => ((88, 92, 104), "--"),
            };
            if line.level == Level::Bad {
                worst = 1;
            }
            let dot = if colour {
                format!("\x1b[38;2;{};{};{}m●\x1b[0m", rgb.0, rgb.1, rgb.2)
            } else {
                word.to_owned()
            };
            println!("    {dot} {:<18} {}", line.label, line.detail);
        }
        println!();
    }
    worst
}

fn restart(path: &Path) -> i32 {
    if let Ok(settings) = Settings::load(path) {
        engine::set_client(settings.client());
    }
    match engine::restart_with_debug_port() {
        Ok(client) => {
            println!("{} relancé avec le port de debug", client.label());
            0
        }
        Err(error) => {
            eprintln!("échec : {error}");
            1
        }
    }
}
