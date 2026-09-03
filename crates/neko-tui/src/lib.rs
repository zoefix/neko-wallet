//! Terminal UI.

pub mod app;
pub mod chain;
pub mod clipboard;
pub mod event;
pub mod input;
pub mod keys;
pub mod nav;
pub mod render;
pub mod send;
pub mod theme;
pub mod ui;

mod run;
pub use run::{run, run_with};
