//! Window helpers. Window properties (size, transparency, always on top, ...) live in
//! `tauri.conf.json`; these modules handle placement and show/hide behavior.
//!
//! This module shares its name with the `windows` crate: refer to the crate as `::windows`.

pub mod mascot;
pub mod popup;
pub mod settings;

pub const MASCOT: &str = "mascot";
#[cfg_attr(not(debug_assertions), allow(dead_code, reason = "used by the popup flow from Task 6"))]
pub const POPUP: &str = "popup";
pub const SETTINGS: &str = "settings";
