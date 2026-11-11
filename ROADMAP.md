# DX RUST roadmap

DX RUST rebuilds the original Deus Ex (2000) in Rust on the Bevy engine, running on the game's own data files.

- **v1.0**: the original game, every mission, played 1:1, on Windows, Linux / Steam Deck and macOS.
- **v2.0**: optional modern improvements on top, each a switch in Settings > Enhancements (Tab in play flips
  between Classic and Modern).

## Where we are (November 2026)
The whole game is implemented: every mission script, map, set piece and ending, and every system (AI, combat,
stealth, skills, augmentations, inventory, conversations, computers, saves, menus, HUD), checked against the
original scripts. Most of v2.0 is in as well, and co-op multiplayer is playable.

## v1.0: what's left
1. **A full playthrough**, M00 to the endings, fixing what comes up. Watch conversation events after mission 1,
   flags carried between maps, followers crossing maps, bosses and killswitch deaths, vehicles and elevators.
2. **Tuning by eye**: water, the intro's logo electricity and mirror floor, letterbox height on wide screens.
3. **Performance**: the render thread (Bevy's per-view command recording) is the limit with Modern and every mod.
4. **Release polish**: Steam Deck at 800p / 60 fps, save round-trip tests on every map.

## v2.0: done
- **Controls**: full controller support, Xbox / PlayStation glyphs, aug wheel, DualSense light bar, triggers,
  haptics and speaker.
- **Interface**: searchable Enhancements menu, Classic / Modern switch, HUD customisation, dialogue hotkeys, codes
  from notes, item rotation and Pack, inventory sort, Field of View, frame rate cap, achievements, Discord presence,
  terminals and keypads used in the world.
- **Gameplay**: GMDX perks, fixes and quality-of-life options, Smarter AI, body alarms, thrown-object noises,
  footsteps by surface, NPC ammo, sprint and stamina, mantling, ladder grabbing, carried cover, bullets through wood,
  smooth lean, melee arcs, melee bash, elemental reactions, Modern Aiming with aim-down-sights and a live scope,
  third person with cover peeking; GMDX NPC combat (strafing, firing positions, grenades, wider alarms).
- **Visuals**: real-time shadows, remastered per-pixel lighting, normal and parallax maps, a material table,
  glowing surfaces that light their surroundings, Modern Water, rain and lightning, clouds, a 3D skyline, fog and
  light shafts, bloom, ambient occlusion, night vision, enhanced effects, modern glass, grass and trees, upscaling.
- **Characters**: skeletal bodies (the Machine God team's rig) with blended animation, talking faces, coat cloth,
  ragdolls you can pose, wounds, fluid blood, hurt poses, aug glows, smooth animation blending, JC's body in first
  person and in mirrors.
- **Sound**: better mixing, occlusion, room reverb, a soundscape.
- **Mods**: Lay D Denton, HDTP, New Vision, Helios, GMDX, Augmentique, Visible Attachments, Precipitation and
  the Parallax Occlusion addon, from the launcher's Mods page.
- **Multiplayer**: co-op for up to 16 (the host drives the story, one shared world, own characters and saves), our
  own lobby server with room codes and a public browser, long codes that work without it, LAN discovery, router
  port opening (UPnP), ready-up, chat and a news feed, mod parity (Classic without mods, Modern with the host's),
  healing each other; deathmatch and team deathmatch on the five original maps with balanced pickups, deathmatch
  characters, weapon-skin unlocks, a scoreboard and match settings; Classic deathmatch (the original multiplayer
  patch's four game types, rules, screens and security).

## v2.0: still to do
1. **Multiplayer**: internet playtests; Classic deathmatch played side by side with the original; the Modern
   multiplayer game (a living level, progression between matches, new modes). Its own plan is
   [MULTIPLAYER_ROADMAP.md](MULTIPLAYER_ROADMAP.md).
2. **Weapon mods shown on the weapon**: scope, laser sight and silencer on the view model and in third person.
3. **Materials for weapons and items**: highlights from the material table (they're lit per vertex today).
4. **Leftovers**: AI sees JC at his body while he peeks from cover; GMDX's commando melee and minimum ranges;
   blinking lights don't light their surroundings; tune glow strength by eye.

Cut: more NPCs and livelier locations, no loading screens, PBR materials, time of day.

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
