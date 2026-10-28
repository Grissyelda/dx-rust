//! Engine-neutral gameplay rules shared by the Deus Ex rewrite and later games.
//!
//! Everything here works in Unreal-style units (Z up, ~52.5 units per meter for Deus Ex) and knows
//! nothing about rendering or a particular level format: the world is reached through traits.

pub mod alliance;
pub mod augs;
pub mod body;
pub mod flags;
pub mod inventory;
pub mod movement;
pub mod reactions;
pub mod skills;
pub mod view;
