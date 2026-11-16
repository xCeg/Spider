//! `SpiderBackground.exe`: Spider's presence without any window (no console either), for the
//! background mode and the start with Windows. Spider's window starts and stops it.
#![windows_subsystem = "windows"]

fn main() {
    let config = spider::config::default_path();
    std::process::exit(spider::background::run(&config));
}
