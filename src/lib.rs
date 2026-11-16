//! Spider: Discord Rich Presence without a token.
//!
//! - [`engine`]: the two transports (Discord's debug port and its local pipe).
//! - [`config`]: the `spider.json` file.
//! - [`runner`]: the loop that keeps the file's cards on the profile.
//! - [`doctor`]: the diagnostic.
//! - [`ui`]: the console window.

pub mod config;
pub mod doctor;
pub mod engine;
pub mod runner;
pub mod ui;
