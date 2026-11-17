//! Mod downloads for DX RUST: the catalog of supported mods and their pages (`catalog`), unpacking archives and
//! self-extracting installers in pure Rust (`archive`), and installing them into the game's Mods folder (`install`).
//! Used by the game's Mods screen and the launcher.

pub mod archive;
pub mod catalog;
pub mod install;
