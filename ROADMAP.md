# DX RUST roadmap

DX RUST rebuilds the original Deus Ex (2000) in Rust on the Bevy engine and runs on the game's own data files. It
has two goals, in order:

- **v1.0**: the original game, every mission, played 1:1, from one Windows exe or a Steam Deck build.
- **v2.0**: optional modern improvements on top. Each one is a switch in Settings > Enhancements, so the original
  experience is always one switch away.

## Where we are (October 2026)

The whole game is implemented and the code builds:

- **Missions:** every mission script (training, missions 1 to 15 and the endgame), the three endings and the
  credits.
- **Content:** every scripted actor class and set piece, and every map of every mission loads (missions are split across several maps each; 83 in all).
- **Systems:** AI, combat, stealth, skills, augmentations, inventory, conversations, infolinks, computers, keypads,
  saves, menus and the HUD, all checked against the original scripts.
- **Travel:** moving between maps keeps the state of hub maps, and the next map is read ahead.
- **Extras:** a launcher, controller support with a radial aug wheel, DualSense light bar, triggers and haptics,
  and support for the Lay D Denton Project mod.

What's left is mostly testing a full playthrough, then tuning and performance.

## v1.0: what's left

### 1. A full playthrough
Play the whole game in order and fix whatever comes up:

1. M00 Training.
2. M01 Liberty Island and UNATCO HQ.
3. M02 Battery Park and Hell's Kitchen.
4. M03 Brooklyn Bridge Station, the Mole People and LaGuardia.
5. M04 Hell's Kitchen again and NSF HQ.
6. M05 the MJ12 lab escape.
7. M06 Hong Kong.
8. M08 the New York return.
9. M09 the Dockyard and the Ship.
10. M10 and M11 Paris.
11. M12 Vandenberg.
12. M14 the Ocean Lab.
13. M15 Area 51, the endings and the credits.

Worth watching along the way:
- conversation events first used after mission 1;
- flags carried between maps and missions;
- followers crossing maps;
- bosses and killswitch deaths;
- vehicles and cross-map elevators.

### 2. Small pieces not ported yet
None left. The credits easter eggs THEREISNOSPOON (the Matrix view) and IAMWARREN (Warren's EMP field) now work.

### 3. Tuning by eye
- Water: glint, opacity and ripple size.
- The intro: the electricity line on the DX logo, and the mirror floor.
- Letterbox bar height on wide screens.

### 4. Performance
NPC updates, transform and visibility propagation, the extra camera passes, draw-call count, and mirrors (each
reflecting surface is a full extra render).

### 5. Release polish
- Steam Deck tuning at 800p / 60 fps.
- Lighting remaster options, on top of the original look and never instead of it.
- Save round-trip tests on every map: save, load and save again must give the same file.

## v2.0: Phase 0 (done in code, testing on hardware)
- **Controller support:** every action, the menus, inventory and computers. Separate bindings, modern-shooter
  defaults, a radial aug wheel on a held button, and D-pad aug pages.
- **Button glyphs:** Xbox and PlayStation sets in our own Deus Ex style, auto-detected, shown in prompts and the
  bindings screen.
- **DualSense:**
  - over USB and Bluetooth: a health light bar, adaptive triggers for each weapon class, and rumble;
  - over USB only: HD haptics for footsteps, shots, explosions, damage and breaking crates, and infolinks through
    the pad's speaker.

## v2.0: the rest, by effort

### Small (UI, quality of life, toggles)
1. Enhancements menu (scrolling and searchable; done) as the home of every toggle.
2. NPC ammo running out, with reloads and weapon swaps (done).
3. Dialogue hotkeys 1-4 (done).
4. Use consumables straight from the floor when the inventory is full (done).
5. Shell casings and blood that stay (done).
6. Discord rich presence (done).
7. Inventory auto-sort (done).
8. Codes and passwords from notes offered on keypads and computers (done).
9. Debug menu with the original's cheats (done).
10. The original settings screens moved onto the searchable list (done).
11. Use to grab and Jump to let go of ladders (done).
12. Dragon's Tooth energy balancing.

### Medium (systems whose pieces exist)
13. HUD customisation: toggle, scale and fade each part.
14. Bodies, dead or unconscious, raising alarms when found.
15. Thrown objects making noises that guards investigate.
16. Footstep loudness by floor surface.
17. Weapon inertia and head-bob.
18. A better flashlight: a cone with soft spill.
19. Debris and decals kept across map loads.
20. Perks earned with skill points.
21. Bullets through thin wood and glass (high-calibre rounds).
22. Smoother leaning and a peek assist.
23. Melee that sweeps an arc.
24. Weapon mods shown on the weapon: scope, laser sight, silencer and other fitted mods drawn on the view model and
    the third-person gun.

### Large (new rendering, audio or interaction work)
25. Sway-based aiming, a dynamic crosshair and aim-down-sights (optional).
26. Real-time shadows for pawns and movers.
27. New particle effects.
28. Elemental reactions (electricity through water).
29. Mantling and vaulting.
30. Carried heavy objects as cover.
31. **Optional third-person view** with modern third-person shooter controls: an over-the-shoulder camera that
     keeps out of walls, shoulder swap, aim-to-zoom, and leaning and crouching cover. JC's body already exists for
     mirrors, with his animations and the weapon in his hand, so the work is the camera, aiming from it (trace
     from the camera, fire from the gun), the HUD reticle and stealth balance. It's a toggle, and first person stays
     the default.
32. Volumetric fog and light shafts.
33. Better clouds and a 3D skyline.
34. Remastered sound effects and a dynamic soundscape.
35. Sound propagation and room reverb.
36. New outfits found in the levels.
37. Dynamic weather and time of day.

### Very large (needs skeletons, new content, or touches the whole engine)
38. Smarter AI (opt-in), including stealth that respects darkness more than the original's guards do.
39. HD textures and models, PBR and smoother animation.
40. JC's body visible in first person.
41. Diegetic damage, healing and augs shown on the body.
42. Ragdolls and posable models.
43. Cloth and wetness.
44. Computers shown in the world while it keeps running.
45. More NPCs and livelier locations.
46. **Lay D Denton Project support** (done in code; being tested).
47. No loading screens.
48. Co-op and deathmatch multiplayer.

Characters in Deus Ex are vertex-animated meshes without skeletons, which is why items that need bones sit in the
last tier.

## Engine principles
- **Original gameplay:** keep its quirks, but not its engine flaws.
- **Fixed timestep:** simulation runs on a fixed step separate from the frame rate, and rendering interpolates.
  Outcomes match the original at any frame rate.
- **Saves:** they store only what changed from a map's starting state, so save, load and save gives no drift.
- **Hub maps:** they keep their state when you come back, and the next map is read ahead near an exit.
- **UI:** it scales to any resolution and stays crisp.
- **No map-specific shared code:** mission specifics live in mission scripts, as in the original.
- **One flag store:** conversations, triggers, mission scripts and saves all share it, and flags expire by mission
  as in the original.
- **Checked against the original:** every system is compared with the original UnrealScript behaviour.
