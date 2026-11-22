# DX RUST roadmap

DX RUST rebuilds the original Deus Ex (2000) in Rust on the Bevy engine, running on the game's own data files.

- **v1.0**: the original game, every mission, played 1:1, on Windows, Linux / Steam Deck, macOS and Android.
- **v2.0**: optional modern improvements on top, each a switch in Settings > Enhancements (Tab in play flips
  between Classic and Modern).

## Where we are (November 2026)
The whole game is implemented: every mission script, map, set piece and ending, and every system (AI, combat,
stealth, skills, augmentations, inventory, conversations, computers, saves, menus, HUD), checked against the
original scripts. Most of v2.0 is in as well, and co-op multiplayer is playable.

## Classic Mode: what's left
1. **Release polish**: One final tiny bug bash. Anything small noticed along the way.  

## v2.0: still to do
1. **Multiplayer**: internet playtests; Classic deathmatch played side by side with the original; the Modern
   multiplayer game (a living level, progression between matches, new modes). Its own plan is
   [MULTIPLAYER_ROADMAP.md](MULTIPLAYER_ROADMAP.md).
2. Bug bashing, minor aesthetic changes (primarily in the diegetic computer system)
3. Continued optimization work.
   
## Engine principles
- **Original gameplay:** keep its quirks, but not its engine flaws.
- **Fixed timestep:** simulation runs on a fixed step separate from the frame rate; outcomes match the original at
  any frame rate.
- **Saves:** they store only what changed from a map's starting state, so save, load and save gives no drift.
- **Hub maps:** they keep their state when you come back, and the next map is read ahead near an exit.
- **UI:** it scales to any resolution and stays crisp.
- **No map-specific shared code:** mission specifics live in mission scripts, as in the original.
- **One flag store:** conversations, triggers, mission scripts and saves all share it; flags expire by mission.
- **Checked against the original:** every system is compared with the original UnrealScript behaviour.
