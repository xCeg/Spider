//! Spider: Discord Rich Presence without a token.
//!
//! - [`clock`]: the local time ({heure}, {date}, the "time of day" timer).
//! - [`engine`]: the two transports (Discord's debug port and its local pipe).
//! - [`config`]: the `spider.json` file.
//! - [`runner`]: the loop that keeps the file's cards on the profile.
//! - [`doctor`]: the diagnostic.
//! - [`profiles`]: saved configurations.
//! - [`background`]: the windowless mode and the start with Windows.
//! - [`ui`]: the console window.

pub mod background;
pub mod clock;
pub mod config;
pub mod doctor;
pub mod engine;
pub mod profiles;
pub mod runner;
pub mod ui;
