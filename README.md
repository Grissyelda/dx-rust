<p align="center"><img src="assets/dxrust_256.png" width="128" alt="DX RUST icon"></p>

# DX RUST

**The original Deus Ex, rebuilt in Rust on the Bevy engine and played with your own copy of the game. Now with co-op
and deathmatch multiplayer.**

<p align="center"><a href="https://discord.gg/cY9kJ7fETm"><img src="https://img.shields.io/badge/Discord-Join%20the%20DX%20RUST%20community-5865F2?logo=discord&logoColor=white" alt="Join the DX RUST Discord"></a></p>

**[Join the DX RUST Discord](https://discord.gg/cY9kJ7fETm)** to report bugs, suggest enhancements and find
multiplayer matches.

DX RUST is a from-scratch reimplementation of *Deus Ex* (Ion Storm, 2000). It reads the maps, textures, sounds,
music, conversations and class defaults straight from your install and plays the whole game the way it shipped:
every mission, all three endings, the AI, stealth, skills, augmentations, computers, saves, and the original menus
and HUD. It runs natively on Windows, Steam Deck / Linux, Apple Silicon Macs and Android, with no old engine
underneath.

![DX RUST: the training course, playing as the female JC from the Lay D Denton Project](docs/screenshot-training.png)

> **You need your own copy of Deus Ex** (Game of the Year Edition from Steam or GOG). No game files are included
> here or in the releases.

## Features
- **The whole game**, faithful by default: the original rules, numbers and quirks, checked against the original
  scripts. Every modern change is an optional switch in *Settings > Enhancements* (one searchable list).
- **Multiplayer**:
  - **co-op** for up to 16 players through the story, in one shared world;
  - **Deathmatch and Team Deathmatch** on the original's five multiplayer maps;
  - room codes, a public lobby browser, and direct codes that work without any server.
- **Classic / Modern in one key.** Press **Tab** and the screen wipes between the game as it shipped and a modern
  look with every enhancement on (your own mix is remembered).
- **A modern look**: remastered per-pixel lighting with relief maps, real-time shadows, ambient occlusion, bloom;
  rain and lightning storms, volumetric clouds, fog and light shafts; waves with reflections, an open sea, frosted
  glass, mirrors; grass and grown trees; a photo skyline.
- **Modern bodies**: skeletal characters (the Machine God team's rebuilt people) with lip sync and blinks,
  ragdolls, fluid blood, wounds and hurt poses, and JC's own body in mirrors, third person and first person.
- **Modern play**: swaying aim with aim-down-sights, a live-lens rifle scope, third person with cover, mantling,
  sprinting, Smarter AI (GMDX's tougher guards), terminals and keypads used in the world, healing by body part,
  better audio (occlusion, reverb).
- **Controllers**: full gamepad play with remapping, an aug wheel and button glyphs; DualSense light bar, adaptive
  triggers and HD haptics.
- **Mods**: Lay D Denton Project (play as the female JC), HDTP, New Vision, Visible Attachments, Augmentique, GMDX
  perks, Helios, Precipitation and Parallax Occlusion Mapping, as many at once as you like.
- **Extras**: 50 achievements, saves per character, FSR 1 upscaling, a Field of View slider, a frame-rate cap,
  Discord Rich Presence, and a debug menu with the original cheats.

## Getting started
Download your platform's file from [Releases](../../releases).

### Windows
1. Unzip `dxrust-windows.zip` anywhere and run **dxlauncher.exe**.
2. It finds your Deus Ex folder (or click *Game Folder...*). Choose a display mode and resolution, then **Play**.

### Steam Deck / Linux
1. Install *Deus Ex: Game of the Year Edition* through Steam (you don't have to run it).
2. In Desktop Mode, extract `dxrust-steamdeck.tar.xz` (for example to `~/Games/dxrust`) and run `dxlauncher` once
   to check that it found the game. Steam libraries and SD cards are searched.
3. Add `dxrust.sh` to Steam as a *Non-Steam Game* to play from Game Mode, with Steam Input's *Gamepad* template.

### macOS (Apple Silicon)
1. Unzip `dxrust-macos.zip` and move *DX RUST* to Applications. The first time, right-click it and choose *Open*
   (or run `xattr -dr com.apple.quarantine "/Applications/DX RUST.app"`).
2. Get the Windows game files (they're the same on every platform) into `~/Games/Deus Ex`:
   - **Steam:** run `open steam://open/console`, type `download_depot 6910 6911`, and leave or move the
     `depot_6911` folder it names (DX RUST finds it).
   - **GOG:** `brew install innoextract`, then `innoextract --gog -d ~/Games/Deus\ Ex setup_deus_ex_goty_*.exe`
     (use the `app` folder if `System` is inside it).
3. Open *DX RUST*; the launcher finds the folder. Choose a resolution and **Play**.

Settings and saves are in `~/Library/Application Support/DX RUST`. Resolutions are in points (Retina draws twice
the pixels each way), so pick a smaller one for Modern if the frame rate is low. Turn on *Use F1, F2, etc. keys as
standard function keys* for the aug keys.

### Android (Classic)
For 64-bit phones and tablets on Android 10 or newer with Vulkan, in Classic (no mods or Modern features).
1. Copy your whole *Deus Ex* folder (with `System`, `Maps`, `Textures`, `Sounds`) to the phone or SD card.
2. Install `dxrust-android.apk`, start it and allow *All files access*: it searches the phone and starts the game.

**Touch:** the left half is a floating move stick; drag on the right half to look, tap to fire, hold still to keep
firing. Use, Jump, Duck, Reload and Zoom sit above the belt, Lean on the left edge. Tap the HUD: belt slots, aug
icons, the health figure (Health), the compass (Goals), the ammo display (change ammo). In menus a finger is the
mouse (long press = right click, two fingers scroll). *Settings > Enhancements > Touch* has look speed, button size,
opacity and optional gyro aim. Bluetooth and USB controllers work as on the PC, and Android joins and hosts Classic
co-op. Saves are in `Android/data/com.dxrust.game/files`.

### Multiplayer
Choose **Multiplayer** on the main menu.
- **Host Room**: Co-op, Deathmatch or Team Deathmatch; Classic or Modern; public or private. You get a short code
  (`DX12`) and a long code (`ABCDE-FGHIJ-KLMNO`); everyone marks *Ready*, then the host presses *Start Game*.
- **Join Room**: type either code and pick or make a multiplayer character. **Lobby Browser** lists public rooms.

**Co-op**: the host plays the story (conversations, scripts and travel follow the host); enemies, doors, computers
and kills are shared; each player keeps their own inventory, skills and augmentations. Fallen players get up near
the host (on Hard and Realistic, without their inventory). Use a medkit on another player to heal them. **Y** chats,
hold **Tab** for the scores. Classic rooms switch your mods off; Modern rooms need the host's mods.

**Deathmatch / Team Deathmatch** (up to 8 v 8) play in Modern with Augmentique. Make a *Deathmatch Character* (body,
face, outfit); kills unlock weapon skins. The host sets the map and the frag and time limits.

**No server needed:** the long code holds the host's address. If the room screen says the router didn't open the
port itself, forward **UDP 27972** to the host (or turn on UPnP). Players on the same network find the host on their
own. -- You can also use Radmin VPN or Hamachi to skip port forwarding, similar to playing Minecraft with friends.

**Running your own lobby server:** `dxlobby` (the `dxlobby` crate) is the lobby with a small control window. For a
server with no graphics stack, build it without the window (`cargo build --release --no-default-features` in
`crates/dxlobby`): it then just runs the lobby (port 27962 TCP and UDP), logging to stderr and `dxlobby.log`.
`dxlobby status`, `rooms`, `reset`, `stop` and `upnp` talk to a running server; `--port`, `--admin-port`, `--log PATH|none`,
`--name` and `--key-file` set it up. Players point the game at it with `LobbyServer=host:port` in `launcher.txt`.

### Mods (optional)
1. Unzip each mod into its own folder:
   [Lay D Denton Project](https://www.moddb.com/mods/the-lay-d-denton-project),
   [HDTP](https://www.moddb.com/mods/project-hdtp),
   [New Vision](https://www.moddb.com/mods/deus-ex-new-vision),
   [Augmentique](https://www.moddb.com/mods/augmentique),
   [GMDX](https://www.moddb.com/mods/gmdx) (Augmented Edition), Visible Attachments, Helios, Precipitation, and the
   [Parallax Occlusion Mapping](https://www.moddb.com/games/deus-ex/addons) addon (*Half Size-Full Depth*).
2. On the launcher's **Mods** page, click *Mod Assets Folder...* and choose the folder that holds them all (or set
   each mod on its own button; right-click turns one off).
3. Lay D Denton: on a new game, every other portrait is the female JC. HDTP, New Vision and Helios have switches in
   *Settings > Enhancements > Mods*.

Nothing in your Deus Ex install is changed and no mod files are included. GMDX's perks are rewritten in Rust and
read their data from your copy of the mod.

## Building from source
You need Rust (stable, 1.95 or later) and a C compiler: the Visual Studio C++ build tools on Windows, the Xcode
command line tools (`xcode-select --install`) on macOS. On Linux, also:

    # Debian / Ubuntu
    sudo apt install build-essential pkg-config libwayland-dev wayland-protocols libasound2-dev libudev-dev libxkbcommon-dev
    # Fedora
    sudo dnf install gcc pkgconf-pkg-config wayland-devel wayland-protocols-devel alsa-lib-devel systemd-devel libxkbcommon-devel
    # Arch / SteamOS (desktop)
    sudo pacman -S --needed base-devel pkgconf wayland wayland-protocols alsa-lib systemd-libs libxkbcommon

Then:

    cargo build --release -p dxview                                       # the game
    cargo build --release --manifest-path crates/dxlauncher/Cargo.toml    # the launcher
    target/release/dxview "C:\path\to\Deus Ex"                            # or run the launcher

- **macOS app:** `./BUILD_MAC.sh` makes `dist/DX RUST.app` and `dist/dxrust-macos.zip` (the first build takes
  10-30 minutes).
- **Linux / Steam Deck archive:** `tools/release/build_linux.sh` (cargo-zigbuild, glibc 2.35).
- **Android APK:** `tools/release/build_android.sh` (Android SDK + NDK and `cargo-apk`).
- **All releases from Windows:** `BUILD_BUILDER.bat` builds and opens `DXBuilder.exe`, the Release Builder. It
  makes the Windows zip on your PC, the Linux archive in WSL (its Setup tab installs WSL and the tools) or on
  GitHub, and the macOS and Android builds on GitHub (`.github/workflows/release.yml`, from the code on GitHub, so
  push first). It can then upload them to a draft GitHub release with the changelog's notes. Without the repository
  secrets `ANDROID_KEYSTORE_B64` / `ANDROID_KEYSTORE_PASSWORD`, every APK gets a new key, so installing a newer one
  means uninstalling the old one first.

| Crate | What it does |
| --- | --- |
| `unreal-pkg` | Reads Unreal Engine 1 packages: names, exports, properties, textures, sounds, meshes, class defaults, mod folders |
| `dx-level` | Builds a map: BSP geometry, lightmaps, collision, actors, navigation, conversations |
| `imsim-core` | Engine-neutral rules: movement physics, flags, inventory, skills |
| `imsim-audio` | A software mixer: positional sounds and tracker music with section jumps |
| `imsim-net` | Multiplayer: the lobby server, encrypted game links, NAT punching and UPnP |
| `dxview` | The game itself, on Bevy |
| `dxlauncher` | The launcher (egui) |
| `dxlobby` | The lobby server, with a control window (egui) or headless |
| `dxbuilder` | The Release Builder (egui) |
| `dxandroid` | The Android app around `dxview`; `vendor/winit` is winit with an Android controller hook |
| `dxtool` | Command-line package inspector |

## Status
The whole game is implemented. Before 1.0: a full playthrough with fixes, tuning by eye, more render-thread
performance work (Modern with every mod runs about 70 fps on a Ryzen 5 3600 and RTX 3050), Steam Deck tuning, and
save round-trip tests on every map. The full list is in [ROADMAP.md](ROADMAP.md).

If your frame rate sits at exactly 30, turn off *Vertical Sync* (Settings > Display). On a CPU with many cores, also
try *Settings > Enhancements > Display: CPU Worker Threads*. Bug reports are very welcome: say which map you were on
and what you did.

## Legal
- DX RUST is an unofficial fan project, not affiliated with or endorsed by Eidos, Square Enix, Embracer or Ion Storm.
- *Deus Ex* is a trademark of its owners. All game content belongs to them and is read from your own copy at run
  time.
- The Lay D Denton Project is used with its authors' permission. HDTP, New Vision, Visible Attachments, Augmentique,
  Helios, Precipitation and GMDX belong to their authors. No mod's files are redistributed here.
- The rebuilt human bodies, rig and animations in `assets/rig` are the Machine God team's, included with their
  permission (credit them if you reuse them). They contain no Deus Ex data.
- DX RUST's own source code is licensed under the **GNU GPL v3.0 or later** (see [LICENSE](LICENSE)).

## Credits
- Ion Storm Austin, for Deus Ex.
- The Lay D Denton Project team, for the mod and permission to support it.
- The HDTP team, the New Vision team, and the authors of Visible Attachments, Augmentique, Helios, Precipitation and
  GMDX (CyberP, and Sarge and the Augmented Edition team), for their mods.
- The Machine God team, for the rebuilt human bodies, rig and animations behind Skeletal Characters.
- Joe Wintergreen, whose lipsync mod's timings Smooth Lip Sync follows.
- Contributors: JohnUzoka (the Performance settings, Lean Rendering / Lean Updates, the FPS counter, Open Sea and many
  speed-ups, from his Nintendo Switch port) and poltak (macOS support).
- The Bevy, egui, gilrs, hidapi and Avian projects, and the Rust community.
