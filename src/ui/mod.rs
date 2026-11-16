//! Spider's console window: the animated logo, the menu, the configuration wizard, the live
//! dashboard and the diagnostic.

mod app;
pub mod logo;
pub mod paint;
mod preview;
mod term;
mod wizard;

pub use app::run;
