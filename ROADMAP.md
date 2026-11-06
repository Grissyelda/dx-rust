# DX RUST roadmap

DX RUST rebuilds the original Deus Ex (2000) in Rust on the Bevy engine and runs on the game's own data files. It
has two goals, in order:

- **v1.0**: the original game, every mission, played 1:1, from one Windows exe or a Steam Deck build.
- **v2.0**: optional modern improvements on top. Each one is a switch in Settings > Enhancements, so the original
  experience is always one switch away.

## Where we are (November 2026)

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
Latest pass (measured on an RTX 3050 with every Modern option and all the mods): the frame was held up by the
render thread, which drew 29 views a frame - five cameras and 24 shadow-map faces. Real-time shadows now use one
aimed spot view per light, and only for lights with someone in them, which took the frame from about 30 ms to 21 ms
(32 to 47 fps). Rain is built on the GPU from a small data texture (it re-uploaded about 2.5 MB of mesh a frame),
and water reflections redraw every other frame.
CPU pass (November 2026, after a report of poor frame rates on a Xeon E5-2680 v4 with an RTX 2070, default settings):
the GPU was nearly idle; the main thread and the render thread each took 12-14 ms on a Ryzen 5 3600, so slower
cores fell under 60 fps and VSync halved that to 30. Fixed: collision tests skip what they can't reach (the AI cost a
third of before), the ragdoll physics sleeps while nothing falls, static meshes (water, the open sea, level surfaces)
no longer have their bounds recomputed every frame because their materials animate (1.6 million vertices a frame),
the sky room is drawn by the world camera instead of its own, the worker pool stops at 10 threads on many-core CPUs,
render passes stay on the render thread, unused Bevy plugins are off, the depth pre-pass waits until Modern Water needs it,
and the release build is optimised as one program. On the Ryzen 5 3600 the main thread went from 13.9 to
7.9 ms a frame and the render thread from about 12 to 9.4 ms; default settings run at 10.2 ms (98 fps, was 72 fps
before the last fixes). Modern with every option and mod: about 24 ms (41 fps), all of it the render thread (pass
recording and Bevy's per-view preparation). GPU-driven pass: bindless materials for the world and the rebuilt
characters (multi-draw), GPU culling of the level's surfaces, rain moved by the shader, water and panning textures on
the GPU's clock, materials no longer rewritten every frame, blob shadows in one mesh: about 1,000 draw calls to 250
and Modern at about 14 ms (70 fps), Classic 11 ms (92 fps). What's left is mostly Bevy's own per-frame render
bookkeeping (per-phase bind groups, command recording and submission). Allocation and code-health pass:
mimalloc, per-system allocation counts in profile runs, and the game's own per-frame allocations cut from thousands to a few hundred at most
(JC's mesh lookup was copying HDTP's whole override list twice a frame; snapshots, queues, HUD keys and string tests
now reuse buffers or borrow). Next: Bevy's per-view render bookkeeping (bind groups and buffers rebuilt each frame),
the weapon view's own pass, the Update schedule's scheduling overhead (about 160 small systems), and splitting the
NPC component and its update into smaller components and systems (poison and bleeding are their own component and
system; the update's damage, sight, hearing and state code are separate functions now; next: perception, reactions,
combat, orders and movement as components and systems of their own).

Since then (November 2026): procedural textures worked out on a background thread, skeletal people posed less
often by distance, the sound mixer no longer holding the game up (the game queues its changes), the render world's
schedule run in order on the render thread and the worker pool sized to the processor's cores, and invisible
mask sections (the NSF's masks, glasses slots) no longer drawn - see-through draws on the Island's dock from about
110 to 25. On a Ryzen 5 3600 with an RTX 3050: Classic about 8 ms a frame (125 fps), Modern with every option and
mod about 15 ms (planar water reflections on; screen-space is cheaper).

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
12c. Item rotation in the inventory grid, a Pack button and Shift-click to the belt, with controller shortcuts on
    the persona screens (done); a Field of View slider (done).

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
    firearms only; the sights line up on the original weapon models as well as HDTP's, and a scoped rifle's zoom
    is a live lens in front of the eye - Diegetic Scope).
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
40. JC's body visible in first person (done in code, needs Skeletal Characters): look down to see his stomach,
    coat and legs, as Half-Life mods show the body; the head, arms and chest are left out of it (the view model's
    arms are the arms) and it is hidden while crouched.
41. Diegetic damage, healing and augs shown on the body (done in code): wound marks that ride the bone they hit,
    hurt poses (a hanging arm, a stiff-legged limp), aug glows where the augs sit, a healing glow and screen pulse,
    and diegetic healing - hold a medkit, look at the limb to treat and use it there (anywhere else treats the head).
42. Ragdolls (done in code, with the Avian physics crate): the killed and knocked out go limp at once, pushed by the
    hit and carried by their run, on 11 jointed capsules against the level's real geometry; frozen once still.
    JC falls the same way when he dies.
    Explosions still gib, now with Fluid Blood: droplets that fly and splatter, blood that runs down walls and
    pools spreading under the dead. Still to come: posable models.
43. Cloth (done in code): coat tails ripple and settle around the legs (damped spring bones held within a few
    degrees of the animation and pushed out of the legs). Wet bodies in the rain were tried and dropped (everyone
    came out shiny and smooth).
44. Computers shown in the world while it keeps running (done: Terminals In The World - the view moves in to the
    monitor, the security terminal unfolds, the original interface shows on the screen and the mouse works on it;
    keypads are pressed button by button with a small display above; move the mouse away to step back).
46. **Mod support** (done in code; being tested): the Lay D Denton Project, HDTP, New Vision, Visible
    Attachments, Augmentique (outfits and weapon skins) and GMDX: Augmented Edition (perks, bugfixes and
    quality-of-life options), any number at once from the launcher's Mods page.
48. **Multiplayer** (co-op playable, in testing): a new mode of our own rather than the original's, from Multiplayer on the main menu
    (the online services connect only then, with a "Connecting to online services" prompt):
    - **Host Room** makes a four-character room code (letters and digits without look-alikes) and a room screen
      to pick: co-op (first) or deathmatch (second), public (in the lobby browser) or private (code only), up to 16
      players (fewer recommended for co-op), Classic or Modern for everyone (the Classic / Modern switch is off in
      multiplayer, so every player sees the same world), and enemies tougher with more players (a toggle).
    - **Join Room** asks for a code; **Lobby Browser** lists the public rooms (mode, look, players out of 16,
      ping), with a filter toggle over each column and Refresh.
    - **Online services**: our own lobby server (room codes, the public list, finding the host), run from the
      server controller on the host machine (start, stop, restart, reset, start with Windows; it keeps running in
      the background). Game traffic goes player to player with the host's game in charge (it decides hits, so a
      client can't cheat them), encrypted, tuned for latency; a relay through the lobby for players whose routers
      block direct connections.
    - **Co-op**: the host drives the story - their save, their conversations (the others see them talk in real
      time while they keep playing) - and everyone shares one world: the same enemies, doors and alarms, a kill
      counts for all. Loot is per player (everyone can take their own copy of what a crate drops). Each player is
      their own character: made with the new-game screen (skin, male or female - the host's choices don't change
      theirs), shown with their own animation, skin and aug glows, and kept in multiplayer saves of their own
      (inventory, skills, augs) to load or start fresh when joining.
    - **Deathmatch**: the original's rules (DeathMatchGame / TeamDMGame, the UNATCO / NSF / MJ12 multiplayer
      classes, its HUD and messages) ported like the rest, on the five maps the game ships (DXMP_Area51Bunker,
      DXMP_Cathedral, DXMP_CMD, DXMP_Silo, DXMP_Smuggler); how its augs and skill awards sit with Modern is open.
    Done: the lobby server and its protocol (imsim-net: room codes, the public list, UDP hole punching and an
    end-to-end encrypted relay), the server controller (dxlobby), the menus, and first playable co-op (netplay:
    encrypted UDP links, the host's pawns / doors / flags / events / conversation lines / travel sent to the
    players, players' bodies as targets for the host's NPCs, hits and damage both ways, respawn near the host,
    multiplayer characters, tested with two copies of the game). Next: playtests over the internet, the host's
    goals and notes for everyone, alarms / cameras / turrets seeing every player, decorations and dropped items
    shared, players' aug glows and flashlights, then deathmatch proper.

(Items 45, more NPCs and livelier locations, and 47, no loading screens, were cut.)
49. **Remastered lighting** (done in code; an Enhancement, in the Modern preset): every BSP surface relit per pixel
    by its three strongest lights, with their real falloff and the angle they strike at, while the bake's shadows
    stay. The bake already knows, for every lightmap texel, whether each lamp reaches it; the shader takes each of
    those lights' flat baked term back out of the lightmap and puts it back worked out over the surface's relief
    normal, with a highlight by the surface's material. Dropping the lightmaps outright, as first planned, let
    lamps shine through walls: the editor placed its lights for Unreal's baker, which traced their shadows.
    A global gain and an added-light term are there for tuning; per-zone ambient and per-map overrides are not
    done yet.
50. **Normal and parallax maps** (done in code; Relief Maps setting: normal maps, or with parallax): the Parallax
    Occlusion Mapping addon's normal / height maps for about 1,100 textures (shipped with the release, used from the
    Parallax Maps mod folder, or from the game's Textures folder where the DX11 renderer reads them; only its
    `.bump` / `.height` files are read, never its colour replacements, so it works beside Helios and New Vision),
    and generated maps for the rest (height from luminance) on an allow list: brick, stone, concrete, planks, tiles,
    grates, metal panels. Signs, screens, lamps, windows and effects stay flat.
51. **Material table** (done for level surfaces): roughness, highlight strength and metalness by texture name
    (glass, polished metal, painted metal, tiles and marble, wood, plastic, stone and concrete), so metal, glass and
    tiles catch the light. Still to come: weapons and items (their light is worked out per vertex on the CPU) and
    reflections from an environment map or screen-space reflections.
52. **Emissive surfaces** (done in code; Glowing Surfaces setting): surfaces drawn full bright (light fittings,
    screens, signs) and textures named as lamps, screens or neon glow past white, so Bloom gives them a halo.
    Still to come: lighting their surroundings themselves.

Also done: **achievements** - the fifty of Deus Ex Remastered with our own icons, an Achievements screen off the
Logs screen and another on the main menu (which characters unlocked what), an unlock notice in the HUD, spoilers
hidden until unlocked, kept per character, with the endings shared by every character.

53. **Skeletal characters** (done in code, an Enhancement in the Modern preset; the Machine God team's rig and
    rebuilt bodies, shipped with the game with their blessing and credited in the credits): rebuilt male and female bodies
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
    Done: twelve bodies baked and packed small enough to ship inside the exe (about 3 MB), every person on those
    meshes drawn with them in their original skins, moves cross-faded, lip sync / blinks / glances / brows on the
    face's blend shapes, the weapon in the rebuilt hand, lit by the same lights as before, and JC's mirror and
    third-person body. Mods: Helios repaints and Augmentique outfits (JC, Lay D and the people they dress) are worn
    by the rebuilt bodies; HDTP's people are switched off while Skeletal Characters is on. Since then: aim layered
    over walking, running and strafing, heads turning to whoever speaks, strafing both ways, and footsteps and
    melee blows timed from the rig's own motion. Not covered by the rig: commandos, robots, animals and HDTP's
    replacement meshes.

Characters in Deus Ex are vertex-animated meshes without skeletons, which is why items that need bones sat in the
last tier until the skeletal bodies above arrived.

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
