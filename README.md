<p align="center"><img src="assets/dxrust_256.png" width="128" alt="DX RUST icon"></p>

# DX RUST

**The original Deus Ex, rebuilt in Rust on the Bevy engine and played with your own copy of the game.**

DX RUST is a from-scratch reimplementation of *Deus Ex* (Ion Storm, 2000). It reads the game's own maps, textures,
sounds, music, conversations and class defaults straight from your install, and reproduces the original's gameplay
one-to-one:

- every mission, the three endings and the credits;
- AI, combat, stealth, skills, augmentations, inventory;
- conversations, infolinks, computers, keypads;
- saves and the original menus and HUD.

It runs as a native Windows program and on Steam Deck / Linux, with no old engine underneath.

![DX RUST: the training course, playing as the female JC from the Lay D Denton Project](docs/screenshot-training.png)

> **You need your own copy of Deus Ex** (Game of the Year Edition from Steam or GOG). No game files are included
> here or in the releases.

## Features
- **The whole game.** Every map of every mission (the game has 83 map files), every mission script, scripted set pieces and both training courses. Travel
  between maps keeps the state of hub maps, and the next map is read ahead.
- **Faithful by default.** The original rules, numbers and quirks, checked against the original scripts. Every
  modern change is an optional switch in *Settings > Enhancements*.
- **Modern engine underneath.**
  - A fixed-timestep simulation, so play is the same at any frame rate.
  - Resolution-independent, crisp UI.
  - No sound-channel limits.
  - Fast loading.
- **Controller support.**
  - Full gamepad play, with modern-shooter defaults that you can remap.
  - A radial aug wheel and D-pad aug pages.
  - Xbox and PlayStation button glyphs, and snap-to navigation in the menus.
- **DualSense.** Health on the light bar, adaptive triggers for each weapon class, and rumble. Over USB, also HD
  haptics and infolinks through the pad's speaker.
- **Lay D Denton Project support.** Play as the female JC with the community mod's voice, looks and conversations.
  Point the launcher at the mod's folder (see below).
- **Smooth lip sync.** Mouths blend between sounds and close gently, and people blink visibly. The timings follow
  Joe Wintergreen's lipsync mod, and there's a switch in Enhancements to turn it off.
- **Optional extras:**
  - dialogue hotkeys and Discord Rich Presence;
  - inventory auto-sort;
  - keypad codes and logins taken from your notes;
  - using consumables straight from the floor;
  - ladder grabbing;
  - enemies that run out of ammo;
  - casings and blood that stay;
  - a debug menu with the original cheats.

  Every setting, the original ones included, is on one searchable list.
- **Rotating autosaves** on map changes and completed goals.

## Getting started

### Windows
1. Install Deus Ex (Steam or GOG).
2. Download `dxrust-windows.zip` from [Releases](../../releases) and unzip it anywhere.
3. Run **dxlauncher.exe**. It looks for the Deus Ex folder on its own; if it can't find it, click *Game Folder...*.
4. Choose a display mode and resolution, then click **Play**.

### Steam Deck / Linux
1. Install *Deus Ex: Game of the Year Edition* through Steam. Proton installs the files DX RUST needs, and you
   don't have to run the game.
2. In Desktop Mode, download `dxrust-steamdeck.tar.xz` from [Releases](../../releases) and extract it, for
   example to `~/Games/dxrust`.
3. Run `dxlauncher` once to check that it found your game. Steam libraries and SD cards are searched
   automatically.
4. Add `dxrust.sh` (or `dxlauncher`) to Steam as a *Non-Steam Game* to play from Game Mode. Use Steam Input's
   *Gamepad* template.

### Lay D Denton (optional)
1. Download the [Lay D Denton Project](https://www.moddb.com/mods/the-lay-d-denton-project) and unzip it.
2. In the launcher, click **Lay D Denton...** and choose the mod's folder. Right-click it to turn it off again.
3. Start a new game and use the portrait arrows. Every other face is the female JC. Tick *Original Morpheus?* or
   *Original Interactions?* if you want the original versions of those.

The mod's files stay in their own folder: nothing in your Deus Ex install is changed.

## Building from source
You need Rust (stable) and, on Windows, the Visual Studio C++ build tools.

    cargo build --release -p dxview                                       # the game
    cargo build --release --manifest-path crates/dxlauncher/Cargo.toml    # the launcher
    target/release/dxview "C:\path\to\Deus Ex"                            # or run the launcher

On Windows, `BUILD_RELEASE.bat` makes a shareable `dist\` folder.

| Crate | What it does |
| --- | --- |
| `unreal-pkg` | Reads Unreal Engine 1 packages: names, exports, properties, textures, sounds, meshes, class defaults, mod folders |
| `dx-level` | Builds a map: BSP geometry, lightmaps, collision, actors, navigation, conversations |
| `imsim-core` | Engine-neutral rules: movement physics, flags, inventory, skills |
| `imsim-audio` | A software mixer: positional sounds and tracker music with section jumps |
| `dxview` | The game itself, on Bevy |
| `dxlauncher` | The launcher (egui) |
| `dxtool` | Command-line package inspector |

## Status and what's left
The whole game is implemented. Before 1.0:
- a full playthrough, with fixes for whatever it finds;
- tuning by eye (water, a few intro effects);
- a performance pass;
- Steam Deck tuning;
- save round-trip tests on every map.

After 1.0, the v2.0 list: HUD customisation, smarter stealth, perks, new effects, real-time shadows, HD asset
support and multiplayer, among others. Every item stays optional.

The full list is in [ROADMAP.md](ROADMAP.md). Bug reports from playthroughs are very welcome: say which map you
were on and what you did.

## Legal
- DX RUST is an unofficial fan project. It isn't affiliated with or endorsed by Eidos, Square Enix, Embracer or
  Ion Storm.
- *Deus Ex* is a trademark of its owners. All game content (maps, art, audio, text) belongs to them and is read
  from your own copy at run time.
- The Lay D Denton Project belongs to its authors and is used with their permission. Its files aren't
  redistributed here.
- DX RUST's own source code is licensed under the **GNU GPL v3.0 or later** (see [LICENSE](LICENSE)).

## Credits
- Ion Storm Austin, for Deus Ex.
- The Lay D Denton Project team, for the mod and permission to support it.
- The Bevy, egui, gilrs and hidapi projects, and the Rust community.
