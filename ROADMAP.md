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
reflecting surface is a full extra render). Done so far: memory-mapped packages, compressed (BC1) textures on the
GPU, paged GPU vertex animation, weapons prepared at map load so drawing one never stalls, and far fewer meshes
re-uploaded every frame (shiny surfaces, relit decorations, the weapon in hand). Since then: the inventory, persona and
menu screens draw at 1x and copy their unchanging art (about a third of the frame time they used), fire textures
only animate while seen and step on all cores, and rain drops find their landing spots per cell instead of per drop.

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
12. Dragon's Tooth energy balancing (done).
12a. GMDX's bugfixes and quality-of-life options, each a switch in Enhancements (done: skill awards that pay once,
    turrets that ignore animals, blasts that break strong doors, gibbed bodies dropping weapons, Muscle aug jumps
    while carrying, tool counts, item counts, duplicate keys, searched bodies, ammo totals, stack limits, faster
    infolinks, dropping whole stacks, binoculars on the scope key, restricted combat music, no flickering lights).
12b. Classic / Modern: one switch for every enhancement at the top of Enhancements, and in play on Tab with a
    cinematic wipe across the screen, after Halo 2: Anniversary (done; Modern brings back your own mix).

### Medium (systems whose pieces exist)
13. HUD customisation: toggle, scale and fade each part (done).
14. Bodies, dead or unconscious, raising alarms when found (done).
15. Thrown objects making noises that guards investigate (done).
16. Footstep loudness by floor surface (done).
17. Weapon inertia (done).
18. A better flashlight: a cone with soft spill (done).
19. Debris and decals kept across map loads and saves (done).
20. Perks bought with skill points, from GMDX: Augmented Edition (done: 42 perks with their effects; the ones that
    need GMDX-only systems - helmets, the secondary item slot, stamina, the rocket-cam - are left out of the menu).
21. Bullets through thin wood and breakable glass (done).
22. Smoother leaning and a peek assist (done).
23. Melee that sweeps an arc (done).
24. Weapon mods shown on the weapon: scope, laser sight, silencer and other fitted mods drawn on the view model and
    the third-person gun (done, with the Visible Attachments mod).

### Large (new rendering, audio or interaction work)
25. Sway-based aiming, a dynamic crosshair and aim-down-sights (done: Modern Aiming - a slight sway of the gun that
    the crosshair follows, steadier as the weapon skill rises and almost still at Master; aim down the sights with
    firearms only).
26. Real-time shadows for pawns and movers (done: shadows from the map's own lamps, as dark as the light each lamp
    put into the lightmap at that spot).
27. New particle effects (done: Enhanced Effects - impacts by surface, explosion sparks and smoke, barrel smoke,
    blood spray).
28. Elemental reactions (done: shocks run through water, fire spreads by touch, water puts fire out).
29. Mantling and vaulting (done).
30. Carried heavy objects as cover (done).
31. **Optional third-person view** (done: H to switch, J to swap sides, keeps out of walls, closer while aiming,
    shots and use aimed at the crosshair; a free orbit camera with movement relative to it, JC turning to run the
    way you push, strafing while aiming and idle fidgets, or the fixed shoulder view; first person stays the
    default). Still to come: cover leaning and crouching from third person.
32. Volumetric fog and light shafts (done: Atmospheric Fog, Light Halos, Light Shafts - the flashlight, spotlights
    and moonlight through windows as beams in the air - and Local Fog in sewers and bars).
33. Better clouds and a 3D skyline (done: the original skyline photos cut into low-poly buildings at their own
    distances, procedural blocks where a map has no photo, and a slow cloud layer).
34. Remastered sound effects and a dynamic soundscape (done: better playback, speech ducking, a soundscape of
    far-off dogs, gulls and wind from the game's own sounds).
35. Sound propagation and room reverb (done).
36. New outfits found in the levels (done: without GMDX's maps, Augmentique's outfits can be hidden in boxes across
    the original campaign).
37. Dynamic weather (done: rain from the Precipitation mod, per map, steady or coming and going). Time of day is not
    planned.

Also done in this tier, as Enhancements: Modern Water (reflections, real waves, the open sea running on to the
horizon past the map's edge), Glow And Bloom (lenses, lasers, nano canisters, JC's sunglasses), a Modern Vision
Augmentation (image-intensifier night vision with flare, grain and white-hot bodies), Ambient Occlusion and Soft
Shadows with contact hardening.

### Very large (needs skeletons, new content, or touches the whole engine)
38. Smarter AI (done, opt-in): GMDX's AI work - perception tightened by difficulty, guards that may open fire at
    once when they spot or hear you close by, a moment's pause before shooting after losing sight of you,
    suppressive fire where you vanished, worse aim against a moving target, cameras that see farther and tell the
    guards where you are - plus our own stealth: true darkness hides you beyond arm's reach, guards that searched in
    vain stay on edge, call each other over, and notice lights going out and doors opening; optional alert markers
    over their heads. Still to come from GMDX: smarter strafing, melee bashes, NPC grenades, wider alarms.
39. HD textures and models (done with the HDTP, New Vision and Helios mods - Helios dresses the people, HDTP the
    items - switchable in Settings), PBR and smoother animation.
40. JC's body visible in first person.
41. Diegetic damage, healing and augs shown on the body.
42. Ragdolls and posable models.
43. Cloth and wetness.
44. Computers shown in the world while it keeps running (done: Terminals In The World - the view moves in to the
    monitor, the security terminal unfolds, the original interface shows on the screen and the mouse works on it;
    keypads are pressed button by button with a small display above; move the mouse away to step back).
45. More NPCs and livelier locations.
46. **Mod support** (done in code; being tested): the Lay D Denton Project, HDTP, New Vision, Visible
    Attachments, Augmentique (outfits and weapon skins) and GMDX: Augmented Edition (perks, bugfixes and
    quality-of-life options), any number at once from the launcher's Mods page.
47. No loading screens.
48. Co-op and deathmatch multiplayer.
49. **Remastered lighting** (an Enhancement, off by default; the look of the cancelled 2026 remaster's trailer):
    the baked lightmaps dropped and every level surface lit in real time by the map's own Light actors (the
    nearest ones with shadows, the rest unshadowed), so lamps light the walls and floors with real falloff and the
    existing real-time shadows, soft shadows, ambient occlusion and bloom all apply to the lit result. Needs a
    per-zone ambient term and a global gain tuned by walking the maps, since the editor's light placements were
    tuned for Unreal's baker, with per-map overrides where a room comes out wrong. The original lightmap renderer
    stays the default.
50. **Normal and parallax maps** for the level textures: nobody ships them for Deus Ex, so they are generated from
    each texture's own colour (height from luminance, filtered into a normal map) with a per-texture allow list
    (bricks, grates, planks and panels gain relief; textures with painted-in lighting stay flat), feeding the
    real-time lights above. Hand-made normal maps from the HD mods used where they exist.
51. **Material table**: roughness, metalness and reflectance per texture group and per weapon / item mesh (metal,
    glass, wood, skin, cloth), so guns, chrome, wet stone and glass get real specular highlights and reflections
    from an environment map per zone or screen-space reflections, instead of today's flat, matte materials.
52. **Emissive surfaces**: every glowing texture group (lamp fixtures, ceiling panels, console screens, signs,
    alarm lights) marked emissive so it blooms and, with remastered lighting, lights its surroundings itself.

Also done: **achievements** - the fifty of Deus Ex Remastered with our own icons, an Achievements screen off the
Logs screen, an unlock notice in the HUD, kept for the player across saves.

53. **Skeletal characters** (an Enhancement, from a community rig of the Deus Ex people, used only with its
    author's permission and read from the player's own copy, like the mods): rebuilt male and female bodies
    (trench coat, suit, dress shirt, jumpsuit, hoodie, dress, skirt suit, children) on two rigs with 126 deforming
    bones each - 72 of them in the coat tails - and 172 animations: every sequence the game plays, at the original
    frame counts, plus 8-way strafes, cover moves, head look-at, masculine / feminine walks and idles and about 30
    conversation gestures. Faces are 14 blend shapes driven from the rig (the game's own lip-sync mouths, blinks,
    brows, smile, snarl, eyes). Checked: a headless Blender bake gives glTF with a 126-joint skin, the 14 morph
    targets and the actions on the deforming bones, which Bevy loads natively. What it opens up:
    - lip sync that blends mouth shapes and brows / expressions with the dialogue (item 53a);
    - third-person and NPC animation that blends, layers aim over running and turns heads to speakers (53b);
    - JC's own body in first person, his shadow and reflection (item 40);
    - ragdolls on the deforming bones with a physics crate, and spring coat tails (items 42, 43).
    Steps: a bake script per body (Blender, run once on the player's machine or ours), a skinned-mesh path beside
    gpuanim.rs keyed by mesh name, animation notifies (footsteps, melee hits) mapped by frame position, and the
    original vertex meshes kept as the default. Not covered by the rig: commandos, robots and animals.

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
