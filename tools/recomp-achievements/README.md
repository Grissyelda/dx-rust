# Recomp Achievements list for DX RUST

`dx-rust.json` is DX RUST's achievement list in the Recomp Achievements format (events kind), for
RecompLauncher's achievement system (https://github.com/elripalda/RecompAchievements). RecompLauncher hosts
the list; this copy is the one to submit or paste into the developer hub.

The game reports each unlock as an event named after its achievement id (`crates/dxview/src/rlach.rs`, called
from `achievements::unlock`), so the `event` of every entry is our id. Story missions are `progression`, the
three endings are `win`, achievements that a story choice can close off are `missable`.

Keep it in step with `achievements::LIST` when achievements are added or renamed.
