# Changelog

## Unreleased

### Classic multiplayer: the original deathmatch (November 11)
- Host Room now asks for the look first. Classic offers the original game types: Deathmatch, Basic Team DM (every
  augmentation, Advanced skills), Advanced Team DM (skill points and augmentations earned by kills) and Custom Team
  DM; Modern keeps our Deathmatch and Team Deathmatch. Classic matches need no mods (the room turns them off).
- Classic plays by the original multiplayer patch's rules, read from your own install: every weapon, skill and
  augmentation takes its multiplayer numbers; damage lands in three regions (head x2, legs, body) with no difficulty
  scaling; the sniper rifle hits hard only through the scope; you start with a medkit, lockpick and multitool and
  six seconds of protection; kills bring skill points and augmentations; weapons stay where they lie for everyone and
  other pickups come back after their respawn time.
- The original's screens: buy skills with the number keys (Skill menu key, B), "Skills available!", the death screen
  with "Press <Fire> to restart" and the killer's details (L), the centred notices ("You hit your teammate!", "Only 4
  more kills, and X will win the match!"), the original scoreboard and win screen.
- Team games dress everyone as UNATCO troopers or NSF terrorists; in Deathmatch pick JC, an NSF terrorist, a UNATCO
  trooper or an MJ12 agent in Player Setup (Multiplayer menu, or Character in the Host Room).
- Turrets and cameras work as in the original's multiplayer: off until someone switches them on at a security
  computer, then they're his - they never shoot him or his team - and they can't be destroyed.
- Grenade, mine and turret kills now go to whoever threw, planted or owns them.

### Multiplayer groundwork (November 11)
- Map data can now hold per-mode spawn sets, play-area blockers, objective spots, loot tables and codes made fresh
  each match (for the modes to come).
- Deathmatch characters keep an operative profile: credits earned by kills, heals and matches, weapon mastery,
  unlocks and loadouts (the store comes with the new modes). The Record row shows your credits.
- Network budget: `DXVIEW_NETBUDGET=1` prints the bytes sent per message kind every 5 s; big decoration updates go
  in smaller pieces.

### Enemies run out of ammo (November 10)
- "NPC Finite Ammo" is now on by default, in Classic too, as in the original: guards use up the ammo they carry,
  switch weapons when a gun runs dry and draw their knife, baton or crowbar when nothing is left. Existing players
  get it switched on once; turn it off in Settings > Enhancements for the old endless ammo.
- Weapon choice follows the original SwitchToBestWeapon fully: a weapon's damage type against the enemy (EMP and
  scramble grenades only at robots, no gas at robots or at someone already gassed or stunned, explosives preferred),
  the enemy's own weapon range, projectile weapons' real ranges, and bUseFallbackWeapons (Howard Strong fights with
  his knife even with a gun). As in the original, a guard with a loaded gun keeps it at close range: melee comes when
  the guns are empty.

### Display settings in the Display menu (November 10)
- Settings > Display now has Frame Rate Cap, Field of View and Upscaling beside Vertical Sync, and an Advanced...
  button for Anti-Aliasing, HDR Rendering, Real-Time Shadows, Lean Rendering, Lean Updates, CPU Worker Threads and
  the FPS Counter. They're the same settings as on the Enhancements list, which still has them too.

### AI checked against the original, part one (#48, November 10)
- Every ported piece of the pawn AI now carries a marker naming the original function or state label it follows,
  and `tools/ucaudit/ucaudit.py` lines the original UnrealScript (extracted from your own install) up with the Rust
  code: what is ported, what is missing, and which numbers in the original never appear in the port. See the README.
- Gunfights follow the original Attacking state code: a guard asks where to stand after every leg and every shot,
  and keeps asking while its gun isn't ready (between shots, reloading), so it repositions, closes in along the paths
  or takes a random step toward you the way the original does, instead of standing still until it runs dry.
- Guards check the line of fire before shooting (AICanShoot / AISafeToShoot): no shots through a friend, an
  explosive barrel, a tough crate or a solid door; projectile weapons lead a moving target and respect their
  projectile's own ranges (a rocket isn't fired at point blank); they keep shooting at where you really are for the
  few seconds before they lose track of you, as long as the shot is clear.
- Guards reload every clip, as in the original (NPCs still have endless ammo unless "NPC Finite Ammo" is on); the
  reload is when they move.
- Animals take damage like the original's: straight off their health, with no head shots or lost limbs.
- Knocked-out people give the unconscious groan instead of a death scream, and nobody hears it.
- Alarms: a guard who answered one alarm answers the next, a search ends before the klaxon calls it again, and
  unarmed people react to alarms and loud noises too.
- A gassed or stunned guard who is shot snaps out of it and fights; a second prod doesn't stun again; no flinch
  animation plays while gassed, stunned or burning. Paul takes no fire, stun or knockout damage and little from gas;
  Paul, Anna, Gunther, Walton and MJ12 commandos are never gassed or stunned into helplessness.
- Civilians who find a body run away from it (and from the killer) instead of walking up to look.
- Followers (orders Following) keep following: within about 180 units, stepping back after a second when you stand
  right on them, instead of stopping for good the first time they reach you.

### Android: starts minimized (#62)
- On Android the game window now asks for full screen outright, and a log of the start-up (`startup.log`) and any
  crash (`crash.log`) is written to `Android/data/com.dxrust.game/files/` to find out why it minimizes on some phones.

### Fixes from GitHub issues #54-#61 and the list (November 8-10)
- Achievements no longer unlock in deathmatch (#57): nothing is earned in a versus match or on a DXMP map.
- Reset All on the achievements screen (main menu > Achievements) clears every character's achievements, after a
  yes / no box.
- Throw the item in hand with the middle mouse button (#58), as in the original ("Drop item" in Customize Keys): it
  is thrown forward (harder with Microfibral Muscle). The key ring and a carried body can't be dropped this way.
- Dying with an item in hand (medkit, lockpick...) no longer lets you use it with Fire (#60).
- EMP disables tripwires and laser beams (#61): a confused laser or beam trigger trips nothing.
- The spinning logo on the main menu is lit all round (#59): it was lit once as it stood when the menu opened and
  never again as it turned.
- Classic faces aren't dark brown any more: mesh lighting uses Unreal's falloff (bright close in, then a smooth curve
  to the light's edge) and its x1.5 glow, instead of a straight-line fade that left people half as bright.
- Controller: Select closes the inventory instead of closing and reopening it. Under Steam (#54), the arrow / Enter /
  Escape keys Steam Input sends for a DualSense the game already reads directly are ignored while the pad is in use,
  so menu entries no longer skip.
- Android (#55): on phones whose GPU drivers can't cull on the GPU (older Mali and Adreno drivers) the level is culled
  on the CPU as usual - it wasn't drawn at all there - and binding arrays are turned off when the GPU's limit is too
  small for them. The log now prints the GPU's name, driver and limits.
- Mirrored stairs: clipping a reflection a hair past the mirror cut a sliver off the foot of each step standing on it.
  Mirrors with nothing else in their plane are now clipped a hair short of it. BSP faces with a neighbour's corner in
  the middle of an edge (a T-junction) are drawn as a fan round their centre, so the edge can't crack open.

### Fixes from GitHub issues #45-#53 (November 8)
- Guards don't see as far (#47): how visible you are now fades with the square of the distance instead of in a
  straight line, so someone in the Island's night light is picked out from about 1,900 units rather than nearly the
  whole 2,500 of a guard's sight, and from less in the dark. Guards also have the vertical field of view the game
  gives them (160 degrees across, 70 from top to bottom, looking level: they don't see you on a ledge high above them) and
  its smallest size they can make out (one degree across). Set `DXVIEW_SIGHT_FALLOFF=1` for the old fade.
- Guards move during a gunfight (#48): after each shot a guard picks where to stand, as the original does
  (ScriptedPawn.ComputeBestFiringPosition, which only Smarter AI ran before): one farther off than its weapon's range
  (at most 1,000-1,200 units) steps closer, one in your aim may side-step it, and those that sprint (UNATCO troops
  often, NSF now and then) dash sideways - instead of standing and firing until the clip ran out.
- Overheard conversations stop when someone in them turns to fight, raise the alarm, look for someone or flee (#51):
  the original's FirstPersonConversation ends with an AbortConversation when the pawn leaves it, speech included.
- The O2 meter shows only while your head is under water (#52): at the surface it kept counting down (the timer runs
  there, but the original draws the meter only under water).
- A thrown body drops as in the original (#53): half the throw speed, lying the way you face, and it stays where it
  lands. The old strong throw that slides on is the new Modern option "Strong Body Throw" (Enhancements > Gameplay).
- Linux: the window opens centred on the primary monitor and goes full screen on the monitor it is on (#50); it used
  to open at 0,0 (the leftmost monitor) and full screen always went to the first monitor.
- Mirrors: the last of the outside showing along floor / wall corners is gone (#45). Reflections are drawn smaller
  than the screen, so the smooth sampling mixed in a little of what lies past the mirror's clip plane; that empty part
  is now left out and filled from the reflection next to it.
- The lobby server runs headless (#46): `dxlobby` built with `--no-default-features` has no window code (no egui,
  OpenGL or X11) and just serves; `dxlobby status`, `rooms`, `reset`, `stop` and `upnp` control a running server, and
  `--log PATH|none`, `--name` and `--key-file` set it up.

### Conversation Key Light (November 8)
- New in Modern (Settings > Enhancements > Visuals, "Conversation Key Light"): a third-person conversation held
  somewhere dark gets a soft, warm light on the people in it, set on the camera's side, above and a little to one
  side, so faces can be read. It fades in with how dark the spot is (measured like your visibility to guards) and out
  when the talk ends, follows the camera's cuts smoothly, and lights only the people - not the walls - and guards
  don't see by it. JC's own body in Classic conversations now takes dynamic light too (muzzle flashes, this light).
- Talking to someone through the test aid `DXVIEW_GOTO "secs;<Pawn>;frob"`.

### Performance and memory (November 7)
- wgpu's check of every indirect draw is off where it isn't needed: the game names Vulkan outright (Windows when a
  Vulkan GPU answers, Linux, Steam Deck, Android) or Metal (macOS), so Bevy turns the check off as it means to. With
  the default list of graphics APIs, which includes DirectX 12 everywhere, it stayed on: a compute pass per render
  pass, about 200 MB of video memory for its buffers and extra work on the render thread each frame. DirectX 12 (no
  Vulkan GPU, or `WGPU_BACKEND=dx12`) keeps it, as wgpu needs it there.
- The read-ahead copy of the next map (loaded in the background near an exit) waits until the map in play has
  finished loading: started at once, it stacked a second level on top of the first one's upload copies (Liberty
  Island starts beside the boat: the peak dropped from about 1.2 GB of heap to 0.76 GB here). It is let go once you
  walk well away from that exit, and read ahead again when you come back.
- The lightmap's pixels are freed after they go to the GPU, like the textures' (32 MB on Liberty Island), and freed
  memory goes back to the system after each map load.
- Decoded sounds and speech are kept as 16-bit samples, the depth the game's audio has, instead of 32-bit floats:
  half the memory for the same sound (the cache of recent speech holds the same 25 minutes in 64 MB).
- Weapon and debris meshes are decoded once per session instead of each time a character draws a weapon (a dense
  modded model took several milliseconds each time).
- Profiling (`DXVIEW_PROF=1`) reports memory (resident and committed), CPU use in cores, video memory (the
  driver allocator's total and largest allocations, Bevy's textures), what the CPU side keeps of images and meshes,
  and each map's memory. `PROFILE_RESOURCES.bat` runs Liberty Island and the boat to Battery Park with your settings
  and writes it all to `prof_res.log`.

### GitHub fixes (November 7)
- Locks, doors and devices that can't be picked, broken or hacked show a full green bar beside INF, as the original's
  do (their strength is 1.0) (#43).
- The repairbot screen has Recharge before Close, as in the original, and the button bar's filler covers the gap
  after them (#44).
- Damage triggers work as in the original (#39). They were left switched off until something turned them on, but
  the original's start on, so 77 hazards across the game never hurt: the training's toxic pool (10 a second while
  you're in it, on top of the water's own damage), poison gas in the Hong Kong storage and canal and the NYC
  sewers, electrified water in Vandenberg, steam on Brooklyn Bridge Station and in Area 51, radiation in the MJ12
  lab and Area 51, EMP fields and the Vandenberg drops. Valves, switches and panels that turn them off now do
  (UnTrigger actors were missing too), and their state is kept in saves.
- Burning barrels burn you when you bump into them or stand on them (5 a second), as the original's do.
- Mirrored floors no longer show the outside of the level in a thin line along the corners where they meet walls
  and steps (#45): the reflection cut away the bottom 2 cm of every wall standing on the mirror; it now cuts half a
  millimetre.
- Battery Park's harbour shows its water again with Modern Water: the moving surface was see-through over water the
  original draws solid, so the bare harbour bed showed through by the docks. Water by an open-air zone that has no
  sky of its own counts as outdoors (a trace up to the sky decides).
- A chair with someone sitting in it is "too heavy to lift", as in the original (nothing is lifted while something
  rests on it); you could pick up the receptionist with her chair and never put her down.

## v0.12.0

The highlights since v0.11.0 (the detailed notes for each day follow):
- **Multiplayer**: co-op for up to 16 players, with the host's game as the story and one shared world: NPCs, doors,
  switches, alarms, cameras, turrets, mines and bodies. Every player keeps his own character, skills and saves,
  and players can heal each other with medkits. Our own lobby server has room codes and a public browser; long codes
  work without it, players on one network find each other, and the router's port is opened by UPnP. Rooms have
  ready-up, chat and a news feed, and one name per player. Classic rooms switch your mods off while you play.
- **Deathmatch and Team Deathmatch** (up to 8 v 8) on the original's five multiplayer maps, with the original's
  rules and damage from a table. Pickups are balanced so each map has one GEP gun that respawns every two minutes.
  Deathmatch characters unlock Augmentique weapon skins as kill rewards; a Tab scoreboard and the host's Match
  Settings (frag and time limits, what comes next) round it out.
- **Android (Classic)**: an APK for 64-bit phones and tablets (Android 10+) that finds your copied game folder,
  with floating-stick touch controls, a tappable HUD, Bluetooth / USB controllers, optional gyro aim, and Classic
  co-op online.
- **Classic completeness**: the original's datacube window, frob info boxes with lock and bypass bars, the key
  ring's use animation, scroll bars on every persona pane, notes on DataVault images, and the Gore Level and
  Augmentation Display settings.
- **Hazards and the world**: pain zones on every map that switch off as the original's do, water currents, map
  exits that arrive at the right spot, swaying hanging decorations and rolling carts.
- **Combat and animation**: GMDX's NPC combat for Smarter AI (firing positions, strafing, grenades, wider alarms),
  Third-Person Cover, Melee Bash, Smooth Animation, Posable Bodies, and glowing surfaces that light their
  surroundings.
- **Platforms**: macOS on Apple Silicon (thanks to poltak), a Large HUD by default on a Steam Deck, and the
  Release Builder for Windows, Linux / Steam Deck, macOS and Android from one window.
- **Performance**: JohnUzoka's Performance settings from his Nintendo Switch port (Lean Rendering, Lean Updates,
  Anti-Aliasing, HDR, an FPS counter) and his speed-ups. Procedural textures are made off the main thread, sound
  no longer holds up the frame, and the renderer's own bookkeeping runs on one thread.
- **Fixes** from GitHub reports #11, #12, #13, #14, #16 and #37-#40 (training pickups, the key ring, Classic water,
  doors while carrying a body, mouse look, the toxic pool, thrown bodies), plus a burn timer that ran faster with
  more people around.
- Coat cloth is out of the Modern mix; far water in storms is flat.

### Android (Classic) (November 6-7, night)
- **DX RUST on Android**: 64-bit phones and tablets, Android 10 or newer, as an APK (`dxrust-android.apk`), in
  Classic (no mods or Modern features). The first run asks for All Files Access, finds your copied Deus Ex folder on
  the phone or an SD card, and restarts into the game.
- **Touch controls**: a floating move stick on the left half, drag-to-look and tap-to-fire on the right (a finger
  held still keeps firing), Use / Jump / Duck / Reload / Zoom over the right thumb, Lean on the left edge, Menu and
  Items at the top (Chat and Scores in multiplayer). The HUD is tappable: belt slots, augmentation icons, the health
  figure (Health screen), the compass (Goals), the ammo display (change ammo) and the log. In menus and screens a
  finger is the mouse (long press = right click, two fingers scroll), with a button for the on-screen keyboard.
  Android's Back is Escape.
- **Controllers** over Bluetooth or USB, through the same controller code as the PC (our copy of winit hands their
  sticks, triggers and buttons to the game instead of turning them into touches).
- **Gyro Aim** (off by default) and Touch settings (look speed, button size and opacity, vibration) under
  Settings > Enhancements > Touch.
- **Multiplayer**: Android joins and hosts Classic co-op rooms.
- Android defaults: a Large HUD, Lean Rendering and Lean Updates on, HDR off, 60 fps at most.
- The Release Builder and the *Release builds* workflow make the APK on GitHub.
- The game is now a library (`dxview`) that the desktop exe and the Android app both start.

### Deathmatch and co-op fixes (November 6, night)
- **Deathmatch and Team Deathmatch** (up to 8 v 8) on the original's five multiplayer maps (Area 51 Bunker,
  Cathedral, Vandenberg CMD, Missile Silo, Smuggler's Lair), Modern with Augmentique. The original's rules: spawns
  away from enemies, the multiplayer start kit, scoring, frag and time limits, kill lines. Damage from a table
  (headshots with a pistol or rifle kill). Balanced pickups per map: one GEP gun (back 2 minutes after it's taken),
  at most one of each heavy weapon, the rest spread evenly; pickups are shared and respawn for everyone.
- **Deathmatch Character** (Multiplayer screen): name, body (female with Lay D Denton), face, Augmentique outfit,
  record, and a skin per weapon - kills with a weapon unlock its Augmentique skins (10, 25, 50... kills).
- **Scoreboard** on Tab in any multiplayer session: kills, deaths, heals given, streak, ping; teams side by side.
- **One name per player**: a room won't take a second player under a name someone already plays as (the joiner is
  told to pick or rename a character), and the host can't load or make a character under a joiner's name.
- **Match Settings** for the host: map, frag limit, time limit, what comes after a match, team auto-balance; a
  results screen at the end of a match.
- Co-op fixes: floating items no longer jitter; NPCs killed on the host die (and ragdoll) for everyone; mines and gas
  grenades, turrets, cameras, alarms and hacking are one shared world run by the host; on Hard and Realistic a
  fallen player waits 10 s to respawn and the level restarts if everyone is down at once.
- Coat cloth is out of the Modern mix (switched off once); far water in storms no longer shows facets.

### Multiplayer fixes (November 6, evening)
- **Classic rooms with mods installed**: hosting (or joining) a Classic room switches your mods off while you play
  instead of refusing; they come back when you leave. The Look choice says which mods are switched off.
- **Heal each other**: Use on another player with a medkit in your inventory heals him (the medkit's amount with
  your Medicine skill). His name tag shows his health and how many medkits you carry.
- **The host picks his game**: Start Game opens "Host: Choose Your Game" - any character's saves, New Character (the
  character creator), or Play On with the game that's up. The others see the host's chosen name and look.
- Fix: a male JC looked broken (his face on Lay D Denton's body) on the screen of a player who plays Lay D Denton.

### Release Builder (November 6)
- **DXBuilder.exe** (built by `BUILD_BUILDER.bat`) makes the Windows, Linux / Steam Deck and macOS releases from one
  window: Windows on the PC, Linux in WSL (or on GitHub), macOS on GitHub's Macs, with live logs, Cancel, and a
  *Draft GitHub release* button. It replaces `BUILD_RELEASE.bat`.
- `tools/release/build_linux.sh` and `setup_linux.sh` (the Linux build, usable on any Linux too) and the
  *Release builds* GitHub workflow.

### Multiplayer, round two (November 6)
- **Play without the online services**: every room also gets a long code (like `ABCDE-FGHIJ-KLMNO`) holding the
  host's address and his key's fingerprint, so players can join with it even when the lobby server is down. *Copy
  Code* puts it on the clipboard. The game asks the host's router to open its port (UPnP) and says when you need
  to forward it by hand; players on the same network find the host by themselves.
- **Ready-up**: everyone in the room marks Ready, then the host's *Start Game* lights up. The host can remove a
  player.
- **Chat** (Y) and a **news feed** at the top of the screen: who joined or left, who died and who did it, chat lines.
- **Mod parity**: Classic rooms are played without mods; in a Modern room every player needs the host's mods (the
  room says which to turn on or off).
- **Play Offline**: when the online services can't be reached, host and join by long code from the Multiplayer
  screen. After joining, a waiting room shows who's ready.
- **Shared world, round two**: decorations (broken, pushed, carried, thrown) and items dropped in the world are the
  same for everyone; bodies are searched through the host, so each is looted once; joiners see laser tripwires and
  NPC rockets and darts; guards see joiners by their own light, speed, crouch and camo; name tags over players;
  the player list on P; "An alarm was raised!" and how each player died in the feed; a lost host returns you to
  the title.
- `dxlobby` gained *Open port on router (UPnP)*, which forwards 27962 TCP and UDP to the PC running it.

### Combat and animation (November 6)
- **Smarter AI** gains GMDX's combat: firing positions picked between shots, strafing across your aim, sprinting
  sideways and backing off from blades, some guards running round corners gun up, others holding their ground;
  soldiers throw grenades (gas, and for MJ12 also EMP, scramble and LAM) at a player who stays put; alarms are
  heard 3096 units away, soldiers and robots answer any alarm and search where it points.
- **Third-Person Cover**: in third person, Q / E swap shoulders and peek round the wall or corner beside you.
- **Melee Bash** (F): club with the gun in your hands; it can knock a guard out.
- **Smooth Animation**: longer, eased blends between moves for people, JC's body and the weapon in hand, and smooth
  turning; shots and blows stay on the original's frames.
- **Posable Bodies**: take hold of a ragdoll's arm, leg or head with Use and drag it into a new pose.
- **Glowing Surfaces** now light their surroundings: screens, signs and lamps cast their own colour on nearby
  walls, floors and people (baked when the map loads; nothing extra per frame).
- Fix: an alarm box that went off always searches at itself (it kept an old search point).
- The README and the welcome screen explain multiplayer in a few lines.
- ROADMAP.md is now a short list.

### Multiplayer, first playable co-op (November 5-6, night)
- **Multiplayer** on the main menu (Achievements moved below it). Nothing goes online until you press it, then a
  "Connecting to online services..." box (Cancel stops it). Its screens are drawn like the rest of the menus:
  - **Host Room** gives your room a four-character code (letters and digits without look-alikes, like DX23) and
    lets you choose co-op or deathmatch, public or private, 2 to 16 players, Classic or Modern for everyone,
    tougher enemies with more players, and friendly fire (off unless you turn it on).
  - **Join Room** takes a code. **Lobby Browser** lists the public rooms (code, host, mode, look, players, ping)
    with a filter over each column and Refresh.
- **Co-op** with the host as the story: his save, his conversations and his mission scripts. Everyone else loads
  the same map and sees his world as it happens - every NPC where it is and doing what it does, doors, flags,
  lights and sounds set off by switches, conversation lines and infolinks (heard where they're said, written in
  the log), and map changes (everyone travels with the host and arrives next to him).
  - Each player is his own JC (or Lay D Denton, when her files are installed) with his own face, animated from how
    he moves, holding his own weapon, and heard when he fires.
  - Enemies fight every player: a guard who sees you shoots you, wherever the host is. Your shots count on the
    host's side, so the guard you hit turns on you, and a kill is everyone's.
  - Loot is your own: every item and every body's pockets are there for each player.
  - Anyone can use doors, switches, keypads and computers (the host's game does it, so it's the same for all);
    only the host can start conversations.
  - A player who dies gets up again near the host after four seconds - with his inventory on Easy and Medium,
    with nothing on Hard and Realistic.
  - Joining players keep their own characters in multiplayer saves (inventory, skills, augmentations), written
    every 30 seconds and before each map change.
  - The host's goals show on everyone's Goals screen; other players' flashlights light where they look, and
    the augmentations they run glow on their bodies. Security cameras and turrets watch for every player.
  - Leaving (Leave Multiplayer on the main menu, or the host closing the room) takes a joiner back to the title.
  - Classic / Modern is the room's choice for everyone (Tab doesn't switch it while playing together), and the
    game doesn't pause behind the menus.
- **Deathmatch, first step**: the same rooms with players able to hurt each other, respawning away from the
  others, and "X killed Y (n kills)" in the log. The original's deathmatch rules and maps are still to come.
- **Safety and speed**: players connect straight to the host when their routers allow it (the lobby helps both
  sides open the way at once), else through the lobby's relay. Every game packet is encrypted and signed end to
  end with keys made fresh for each room (Noise protocol), replays are thrown away, and only the real host can
  answer a joiner, so nobody in between - the lobby included - can read or change the game.
- **Online services**: `dxlobby.exe` (built by `BUILD_LOBBY.bat` into the `online` folder) is the lobby server's
  controller (the in-game Multiplayer > Online Services screen sets which lobby to use): it starts the server in the background, shows the rooms, and can stop, restart or reset it, open
  its log, or start it with Windows. Players reach it at `LobbyServer=<address>:27962` in `launcher.txt` (forward
  port 27962, TCP and UDP, to the PC running it).

### Frame-rate pass from the render profile (November 5, night)
- The renderer's own bookkeeping runs on one thread instead of being split into many tiny jobs, and the game uses
  half the CPU's threads for its workers (2 to 8; Settings > Enhancements > Display: CPU Worker Threads still
  overrides it). In the side-by-side runs on the RTX 3050 PC each was worth 1.5-2 ms a frame in Modern (17.5 ms
  as it was) and about half a millisecond in Classic.
- Character parts that draw nothing (the blank grey / black / pink mask textures some outfits leave in unused
  slots) are no longer drawn: with NPCs on screen, from about 110 see-through draws a frame to about 25.

### Architecture sweep (November 5, night)
- Sound no longer holds the game up: the game used to wait for the audio thread to finish mixing (music, reverb,
  every voice) each time it started, moved or checked a sound, dozens of times a frame. It now queues its changes
  and the mixer picks them up at the start of its next buffer; the mixer itself no longer allocates memory while
  it mixes.
- The game's largest systems (the AI, weapons, frobbing, menus, the inventory and persona screens, computers,
  security devices, the HUD and the player's movement and level building) are split into named steps, so each
  reads as what it does in order. Nothing should play differently.
- Burning: the player's burn timer ticked once for every living NPC on the map each frame, so with many people
  around you burned out sooner and took damage faster, and on a map with no one alive the fire never went out. It
  now ticks once a frame.
- Small allocation cuts in skeletal characters' aiming, menu buttons and frob checks.
- `PROFILE_RENDER_AB.bat` measures four ways of running the renderer side by side (as now, the render schedule on
  its own thread, six worker threads, DirectX 12) for the next performance step.

### Modern defaults and the Steam Deck HUD (November 5, night)
- The Modern look's default mix (the one you get before you've picked your own) no longer turns on HDTP's people
  or the two-attacker rule for combat music.
- On a Steam Deck the HUD, menus, inventory and computers start at Large HUD Scale (Settings > Display), so they
  read at arm's length. A setting you've already chosen is kept.

### CPU pass and the debug menu (November 5, night)
- Procedural textures (the animated water, fire and electricity pictures) are worked out on a background thread
  instead of in the frame: the Classic water picture alone took up to a millisecond each step.
- Skeletal characters: bones are written only when they move, people further away are posed less often (every 2nd
  frame past 10 m, 3rd past 30 m, 4th past 60 m), and coat cloth no longer works out its bone order every frame.
- NPCs only trace whether you can see them when they need to know (for those that wait to be seen before acting),
  and the guns they hold are relit less often as they walk.
- The Insert debug menu scrolls with the mouse wheel and shows a scroll bar; the wheel no longer changes your fly
  speed or belt item while it is open.
- The PgDn debug readout is drawn in the game's own font as part of the HUD, wrapped to the screen.

### Hazards and the quest rig (November 5, evening)
- Pain zones work on every map, not only the training's pool, and switch off the way the original's do: the NSF
  HQ's halon gas stops when the switches are thrown, the airfield's steam vent when its valve is turned and the
  Area 51 fan shaft once the fan is destroyed (they used to hurt forever). The halon gas's glowing clouds clear with
  it, and any switched-off light loses its glow. NPCs standing in a pain zone are hurt too, as
  ScriptedPawn.PainTimer does. Pain zones switched on or off are kept in saves.
- Water currents push you while you swim (the Hong Kong storage pipes lift you, the training's last pool drifts).
- Map exits that name a spot in the next map ("map#spot", and the Ocean Lab's "map.dx #spot") arrive there instead
  of at the start; the Ocean Lab silo exit to Area 51 named its map in a way the loader couldn't read.
- Hanging things sway: lanterns, hanging meat, shop and pool-table lights, chandeliers, traffic lights and clothes
  racks swing when bumped, shot or blown at, and the ones the maps set to drift in the wind drift. Hanging chickens
  and pigs bleed when shot. On the freighter, every blast swings them and rolls the carts.
- Utility carts roll when pushed, as the original's do: a bump sends them along at almost your walking speed for
  two seconds, with their rattle.
- A quest rig for testing the whole game (Debug Mode, the Insert menu): warp to any map with the earlier missions'
  story flags set, complete every goal, set this mission's flags, and write the goals and flags to quest_state.txt.
  `QUESTRUN.bat` loads every map in mission order unattended and writes questrun.log: each map's mission script,
  conversations, pawns, load time, the goals and flags it set by itself, and anything that went wrong.
- A whole-game audit (`QUEST_GAME=<Deus Ex folder> cargo test -p dxview quest_audit -- --nocapture`) checks the
  install against our code: every name in the original mission scripts, every flag an original class sets, flags
  nothing sets, map exits and the portals they name, events nothing receives, placed classes with their own
  script, locked doors with no way through, zone hazards and conversations without their speaker.

### Performance settings and speed-ups (thanks to JohnUzoka, PRs #17-#36)
From his Nintendo Switch port of DX RUST, one change per pull request:
- New Settings > Enhancements > Performance group: Anti-Aliasing, HDR Rendering, Lean Rendering, Lean Updates and an
  FPS Counter (frames a second and frame time, top right). The first two are on and the rest off by default, so the
  picture is as before.
- Lean Rendering leaves out GPU work that adds nothing to the picture: the level shader is compiled per material with
  only what it uses, the dynamic lights are lit the way Unreal 1 did (drawn into a copy of the lightmap where they
  reach, so the level shader walks no lights), the shadow-light walk on meshes only where shadow lights exist, and
  still decorations that share a material are drawn as batches.
- Lean Updates redraws menus, the cursor, conversation windows and the HUD only when they change, keeps hidden HUD
  windows out of the layout and skips critters nobody sees; the InfoLink portrait is a HUD window of its own.
- Open Sea (Visuals, off by default): without Modern Water, the sea runs on to the horizon in the Classic look.
- Faster everywhere: the BSP sweep walks each node with only its part of the sweep, fewer collision checks and copies
  on the main thread, settings read without the lock, sounds and speech decoded off the frame, the death screen's
  save lookup only after saves change, and the WetTexture / WaveTexture water pictures made at an eighth of the cost
  (the same picture to the bit).
- Controller: separate horizontal and vertical look sensitivity, a light push on the stick walks, and either face
  button goes on in a conversation and picks the highlighted choice.
- Mouse look keeps working after Load Game, after Escape skips a line and on the death screen.
- The profiler (DXVIEW_PROF) has more scopes, a UI and modified-asset census and a mirror line.

### macOS (thanks to poltak, PR #5)
- Apple Silicon Macs run the game natively on Metal. `BUILD_MAC.sh` makes `dist/DX RUST.app`.
- The launcher and the game find the Deus Ex folder in the Steam library, in the folder that Steam's
  `download_depot` command writes, and in the usual folders.
- An app bundle keeps settings and saves in Application Support.
- The mouse is released when the game loses focus, and the window keeps its size on a Retina display.
- Upscaling works without temporal antialiasing on macOS, because Metal can't draw the skeletal bodies with its
  motion vectors. Native AA only sharpens there.
- The README walks Mac users through building the app step by step: Apple's command line tools, Rust, the
  source, `BUILD_MAC.sh`, and opening a copied app.

### Classic mode completeness (November 5, afternoon)
- Datacubes, books and newspapers show their text in the original's information window: the game's bitmap font,
  the HUD window background and border, each paragraph on its own, headlines centred, placed between the InfoLink
  and the belt; the crosshair and the frob box hide while you read, and it closes when you look away (#38).
  Readables that hold a DataVault image now give it to you ("Image ... added to DataVault"), and one with nothing to
  read says "There's nothing interesting to read".
- The death prompt and the FPS counter use the game's bitmap font too; no smooth system font is left on screen in
  normal play (our quick key editor on Home is now a Debug Mode tool; Settings > Customize Keys is the original's).
- The key ring turns in the lock and comes back, as the original's does (UseBegin, then UseEnd) (#37).
- The training can't kill you, as in the original (it keeps your head and torso at 10 or more), and feet stepping
  into a pain zone always hurt at once; under water the toxic pool waits until your breath runs out, as the
  original's script does (#39).
- Persona screens have the original's scroll bars (up / down buttons and a draggable thumb) on every scrolling
  pane, the inventory's item description included; the conversation list scrolls on its own, so older
  conversations can be reached.
- The Images screen's notes work: Add Note, click on the picture, type; click a note to edit it, Delete Note
  removes it, Show Notes hides them. Notes are kept in saves.
- Settings > Game: Augmentation Display (show only running augs) and Gore Level (Low: no blood spurts, drops,
  splats or pools) now do what they say.
- Choosing Training or Play Intro with a game running asks first, as the original does.
- Settings > Colors shows the original's sample HUD window in the chosen theme.

### GitHub fixes (November 5)
- A thrown body flies as the original's flat carcass instead of a standing person's tall cylinder, so it lands on
  the training med room's table (and anywhere with a low ceiling) instead of hitting an invisible wall (#40).
- The key ring turns in the lock when you use it on a door, as in the original (#37).
- The info boxes over doors, keypads and items use the game's own bitmap font with its shadow, darkened
  background and the coloured Lock / Door / Bypass strength bars with the pick or tool count on them (#38).
- The toxic pool in the training works as in the original (#39): it only starts to hurt once your swim timer runs
  out, you swim at the original's (walking) pace instead of about three times faster, and the damage display shows
  how much the hazmat suit absorbs (its font was looked up in the wrong package, so the percentage never showed).
- Things you pick up in the training no longer vanish (#11): the "stand here" triggers (the munitions trooper's)
  never reset once you walked out of them, so the training kept taking your inventory away, the hazmat suit too.
  Every trigger the player walks out of now reacts the way the original's do.
- The key ring is highlighted on the belt when it's in your hand (#12).
- Classic water is see-through again (#13): additive water was being drawn solid, hiding the pool floor and its
  lights.
- Carrying a body, a right click on a door or a switch uses it instead of dropping the body (#14).
- Mouse look no longer stops after dying and loading (#16): when the window couldn't take the pointer for a moment,
  it was never asked again until you clicked.
- Modern Water's screen-space reflections are steadier: rays leave the flat surface and the waves only bend the
  picture, so the grey shimmer of stray samples is gone; bright lamps no longer leave white sparks. Both
  reflection modes are a little stronger.

## v0.11.0

The highlights since v0.10.0 (the detailed notes for each day follow):
- **Skeletal characters and ragdolls**: the Machine God team's rebuilt human bodies on a real skeleton (blended
  moves, talking faces, coat cloth, hurt poses, aiming and looking), ragdolls for the dead and knocked out, fluid
  blood that runs down walls, wound marks and blood on your weapon.
- **Remastered lighting**: per-pixel relighting of the walls with relief maps (the Parallax Occlusion addon, read
  from the launcher's Mods page) and highlights; real-time shadows reworked; emissive lamps and screens; modern
  glass.
- **Modern Water rebuilt**: one wave grid round the eye, storms with whitecaps, indoor water done right, and
  screen-space reflections worked out on the graphics card (Planar is still there as an option).
- **New enhancements**: sprinting with stamina, grass and grown trees, upscaling (AMD FSR 1), Combat HUD fade, live
  terminals in the world, a diegetic rifle scope, diegetic healing on the body, item rotation and Pack, optional
  gameplay fixes, a Field of View slider and a frame rate cap.
- **Achievements**: per character, spoilers hidden, endings shared across characters, a main-menu screen.
- **Saves per character**, and a welcome screen for players of the original.
- **Performance**: GPU-driven drawing (bindless materials, multi-draw, GPU culling), a CPU pass and an allocation
  pass - Modern with every mod went from about 41 to 70 fps on a Ryzen 5 3600 + RTX 3050; Classic from 72 to about
  96 fps at default settings.
- **Mods**: one Mod Assets folder for all of them, Helios texture packs, GMDX perks and quality-of-life options,
  Augmentique outfits.
- Many fixes from playtests and GitHub reports (#2, #4, #6, #7, #8), the training courses (the stealth maze's alarm,
  the toxic pool's radiation, the holograms) and Mission 1 among them.

### Training, holograms and a processor pass (November 4)
- The training's stealth maze works as it should: the soldiers run for the alarm when they see you (failing the
  test), and the reset switch at the top starts it over. Unarmed guards now raise the alarm or flee instead of
  ignoring you.
- The holograms at the end of training show their animated effect instead of flat purple.
- The training's toxic swimming pool hurts again: pain zones deal their damage every second (radiation there),
  more the deeper you are in, as in the original.
- The flashlight and vision augs glow softly through JC's glasses instead of shining two cones from his eyes.
- New Water Reflections setting (Enhancements > Visuals): Screen-Space, the default, reflects what is on screen and
  is worked out on the graphics card from the picture already drawn - the processor no longer draws the whole world
  a second time for the water; Planar keeps the old full reflection. The water also bends what lies under it.
- The level's surfaces are kept on the graphics card only (less memory).

### Welcome screen, frame rate cap and fixes (November 3, night)
- A welcome window on the main menu for players of the original: what's new, what to try, what to look out for,
  and a note that Modern with everything on is still a work in progress for performance. OK closes it; Don't Show
  Again keeps it closed.
- New Frame Rate Cap in Settings > Enhancements > Display: Off, 30, 60, 120, 144, 200 or 240 fps.
- Footsteps no longer splash on every step whenever it rains: the splash comes in as the ground gets wet, only in
  the open, and stays quiet on hard floors.
- The GEP gun's lock-on stops beeping during conversations, cut scenes and at computers.
- Pieces of broken boxes are lit like everything else, including by dynamic lights.
- People and objects are lit only on the sides facing a light, as in the original: the clone in the intro's tank has
  his face in shadow again. Spotlights light objects all round as the original does, which brings back the training
  range's wall switches (they had gone black).
- Third-person crouching: JC sinks into the crouch and holds it, then walks in it; Lay D Denton's crouch is deeper and
  leans forward like the male one.
- Security computers: if you hacked in without Advanced Computer skill the turret choice stays locked, as in the
  original, and now says why ("Needs Advanced"); a turret's attack mode is kept in saves.

### Bug reports, item rotation, blood and achievements (November 3, evening)
- Classic lighting matches the original again where it was too bright, as in the training halls: spotlights now
  light only inside their cones (they lit all round like bulbs), and lights marked as flat or cylindrical work as
  the original's do. Thanks to RevengeRip for the report.
- Bodies no longer drop the moment you pick them up (the click that lifted a body could also throw it). Thanks to
  RevengeRip.
- New Field of View slider in Settings > Enhancements > Display (60-130, default 75 as the original). The weapon in
  hand keeps its own view so wide angles never show the ends of the models, and JC's body in first person stands
  back further at wide angles. Thanks to RevengeRip for the suggestion.
- The hacking window's empty box is now a countdown of the time left before the hack is detected; it turns red and
  blinks in the last five seconds.
- Ragdolls: knees and elbows bend the way the limb points instead of twisting, and hips swing less.
- New enhancement, Item Rotation (Interface; in the Modern mix): items can lie turned in the inventory grid - R or
  a right click while dragging turns one, R turns the selected item where it lies - and pickups that only fit
  turned go in turned. A Pack button turns and rearranges everything to leave the most room, and Shift-click puts
  an item in the first free belt slot. With a controller on the inventory and persona screens: LB / RB change
  screens, Y uses or equips the item under the cursor, LT drops it, X turns what you're dragging.
- Fluid Blood: blood on walls runs down in trickles with a bead at the tip, slowing as it goes, following the wall
  and dripping off its edge; wounds are openings rather than stains and keep bleeding, pouring for a while from the
  dead; blood from point-blank shots and melee spatters the weapon in hand.
- Achievements: those that give the story away are blurred with a lock over them until unlocked; the endings (and
  Area 51) count for every character once any character has reached them, so all three can be had over several
  playthroughs; and a new Achievements screen on the main menu shows what's unlocked and by which characters.

### Sprint, Classic water and a GPU pass (November 3, afternoon)
- New enhancement, Sprint and Stamina (Gameplay; in the Modern mix): your old running pace becomes the normal
  pace, and holding Walk (Shift) or clicking the left stick sprints about 45% faster. Sprinting uses stamina,
  shown as a bar on the right of the body display, opposite the bioenergy bar (the breath bar takes its place
  under water). It comes back slowly once you stop; candy bars, soy food and soda top it up. No sprinting while
  crouched, carrying something, swimming or in the air; crouching keeps its pace. Run it dry and you're winded
  for a while.
- Classic water moves again: the harbour's water picture was only ever drawn on its first frame, and its ripples
  were too weak to see.
- Augmentique outfit boxes open from every side (from some angles they said you couldn't lift them).
- GPU pass: ambient occlusion, bloom and anti-aliasing cost less (ambient occlusion was the most expensive
  single pass of a Modern frame), clouds skip the work where there's no cloud, relief walls use fewer depth
  layers, and the grass only draws the blades in use (it ran every blade's vertices every frame).

### Playtest fixes (November 3, morning)
- Gunther's cell window really is clear now: frosted glass picked up the level's per-surface light data as a colour
  and drew solid white. The frost itself is fainter on panes that are bright all over.
- JC is back in third-person conversations with Skeletal Characters (he had vanished, so Paul talked to thin air).
- Achievement popups show again after a map change (they unlocked but never appeared after the first map).
- The ATM screen is a little taller and covers the blue screen under it.
- Smoother cut scenes and conversation camera moves: reflections are no longer switched off and back on during a
  scene (each time one came back on the whole level was re-filed for drawing, a hitch in the middle of a pan).
  hitch.log now records the frame times of every cut scene and conversation.

### Characters, achievements test and playtest fixes (November 3, overnight)
- Each new character (named on the New Game screen) keeps its own saves, quick save, five autosaves and
  achievements. The load screen lists every character under a heading of their own that opens and closes like the
  Enhancements groups; the in-game save screen shows only the character you're playing, so nobody else's saves
  can be overwritten. Saves made before this are listed together as "(earlier saves)".
- An achievement test run checks that everything each of the fifty achievements waits for exists in the game and
  that every one can unlock. It found No Thanks, Mean Joe waiting on a misspelt flag (never set): fixed.
- Animated screens and monitors (the med lab's heart monitors, news and observation screens, TV noise) animate
  again, with or without New Vision.
- Gunther's cell glass (and other glass under bloom) no longer glows blinding white.
- Modern Water: indoor water - the pool under Castle Clinton, the subway's - was drawn black; it now shows its
  surface, lit by the room's light.
- The ATM's screen fills its recess; computer screens sit in their monitor even when the terminal settled onto its
  desk or another model replaces it.
- Things resting on a moving part ride it: the items on the shelves of Paul's swinging closet walls.
- Outfit boxes drop to the floor.
- Battery Park: the blue laser in the subway switches off once the hostages are out or the terrorists are dead;
  with the Ambrosia tagged, Anna Navarre waits out front of Castle Clinton instead of wherever the fight left her.
- Liberty Island: once the statue is cleared, every turret still standing stands down.
- JC wears the same face in third-person conversations as in play (Skeletal Characters).
- The first-person body no longer vanishes for a while after the third-person camera.


### Deep optimization and clean-up pass (November 2, night)
- Measured on an RTX 3050 / Ryzen 5 3600 with every mod: Modern about 17.5 -> 16.5 ms a frame, Classic 11.5 -> 10.4 ms
  (96 fps); every part of the frame got shorter.
- Frame rate: effect sprites (smoke, fire, rings, bubbles, muzzle flashes, gas clouds) share one material per
  picture and fade by swapping between its steps, so nothing is re-prepared for the GPU every frame; bullet holes,
  blood splats and wounds share their decal materials and the one quad (they batch into single draws); particle
  generators (faucets, drips, steam, sparks) draw as one mesh each instead of one entity per particle; tracers,
  shock rings, sparks and water rings reuse one built mesh instead of allocating a new one per shot; shell
  casings, fragments and other loose objects stop being relit and moved once at rest; animated textures step only
  while something on screen shows them; the water reflection renders only while a water sheet is in view; the
  electricity bolts, blob shadows, coronas and light halos rewrite their meshes in place and only when they changed;
  the fog's drifting density is refilled at most 20 times a second and only near you; the grass material no longer
  changes every frame; hidden menus (spawn menu, key editor) are out of the UI layout; a hundred-odd places that
  wrote the same visibility, node or transform every frame (forcing Bevy to re-propagate whole subtrees) now write
  only on change; the shadow light layering, splash checks, conversation line-of-sight, coronas' sight traces, the
  decoration relighting and the mesh-actor weapon models skip the work when nothing moved; settings read in
  per-pawn code are cached per call site; several per-frame string copies (NPC weapon stats, ammo names, HUD
  texture keys, sound ambients, key dedup) are gone.
- Clean-up: dead code and unread fields removed (old tessellated water, HUD text leftovers, unused struct fields),
  two latent bugs fixed (the spy drone's state and the skeletal body table survived map changes), the clippy
  backlog taken down from ~390 warnings to a few dozen, `Level::level_info()` / `mission_number()` replace nine
  copies of the same scan, `uinode` / `sprite` helper modules.

### Live terminals, controller layout, ladders and playtest fixes (November 2, evening)
- Diegetic Screens: computers are now used like a live machine. The view stays at your own field of view with the
  monitor filling most of it and the room still around it; the mouse moves the terminal's own cursor, which stays
  on the monitor; the terminal shows a desktop (wallpaper, task bar with the user and a clock, status line). Step
  away with Escape, right click or a walk key. Idle terminals near you show a live standby screen.
- Controller Layout: a picture of the pad with what every button does (Settings > Enhancements > Controller), shown
  once on Liberty Island the first time you play with a controller.
- Ladders (Grab Ladders on): looking at a ladder shows "Climb Ladder" with the key or button; use it to take hold.
- Conversations with many choices (Gunther's weapon request) show every choice.
- Newspapers and other flat two-sided items are no longer black.
- The riot prod glows in your hand (Emissive Surfaces).
- The helicopter outside UNATCO HQ no longer repeats its take-off; throwing things near friendly people no longer
  sends them running to investigate (Manderley left his office).
- New Vision / HD monitors and screens animate again; ladders and vent grates line up with Relief Maps on, and
  guards see through grate vents.
- Battery Park: the grey slab in front of the harbour wall is gone (the mist there now thins out as it should);
  no more ring pattern in the clouds overhead.
- Grate and chain-link vents are see-through again: you can look between the bars into the duct behind (a solid
  face hidden in each vent cover's brush closed it off).
- JC no longer turns black in a mirror when standing close to it (people and objects against a wall are lit right).

### Faster loading and frame rate, water crests, glass, hostages (November 2, afternoon)
- Frame rate: water reflections draw the level, the sky and JC but not every decoration and person (the biggest single
  cost of the Modern look); reflections and the light shafts are cheaper; ragdolls are capped at six at once near you,
  so finishing Liberty Island (every terrorist goes down at once) no longer drops to a few frames a second; mirrors
  render at a lower resolution.
- Loading: textures and relief maps are prepared on all CPU cores, grass is laid out in the background and the
  conversations are read once while the menu is up instead of on every map. With New Vision, Battery Park's relief
  maps alone took seconds.
- Water: the waves are one smooth surface round you instead of many separate water faces, so no seams open between
  them; out at a distance the swell eases off so wave crests no longer stand up as rows of pale triangles.
- UNATCO HQ doors have their glass windows again (a solid face was drawn over each pane), and Modern Glass
  (Settings > Enhancements > Visuals, on in Modern) shows panes as frosted glass instead of a faint smear.
- Battery Park subway: the rescued hostages can be talked to (cowering people could never start a conversation).
- Third person: JC's glasses stay on his face. First person: the coat keeps out of your legs.
- Diegetic Scope: the rifle's own lens no longer cuts in front of the scope picture.
- Diegetic ATMs show their screen only in the machine's screen; diegetic computers hide their see-through panels so
  the screen is readable.
- New Vision textures keep their see-through parts: ladders and vent grates are no longer filled in black, and guards
  can see you through chain-link vents.
- Battery Park: the blue light at the start is no longer blinding. Burning barrels have flames, sparks and a smoke
  column (Enhanced Effects).
- Achievements: Rescue the Lady unlocks as soon as the statue is cleared; unlock notices wait until a map has loaded.
- Out-of-ammo troops with knives who can't reach their target stop standing there and take cover.

### Bone bodies, trees, glowing weapons, far grass (November 2)
- Skeletal people: the arms and chest take an aimed pose over the legs while running or walking in a fight and turn
  toward the target; heads turn to whoever is speaking in a conversation; strafing to the right plays a right
  strafe (it was always the left one); footsteps come when a foot actually comes down; a melee blow lands at the
  fastest point of the swing.
- Coats no longer shake or jerk (the coat fought the legs where the animation itself puts it inside them, and high
  frame rates skipped simulation steps), in first and third person.
- Trees: trunks bend, flare at the foot and are knobbly instead of straight poles; crowns are a few clumps of leaves
  instead of one ball, smaller leaf clusters; each tree is drawn in where a wall, pole or lamp stands close so it no
  longer cuts into what's round it.
- Grass: a second, sparse layer of wider grass carries on beyond the full grass (out to 58 m on High, 34 m on Low)
  and fades in as the near blades thin out, so the lawn no longer ends in a line you can see.
- Glowing weapons and items: with Glow And Bloom, the Dragon's Tooth blade, the riot prod's and EMP grenade's arcs,
  the plasma rifle's coils and the ambrosia vial glow; the Dragon's Tooth lights its surroundings while in hand, as in
  the original.
- Material Properties (Settings > Enhancements > Visuals, on in Modern): the weapon or item in your hands catches the
  light by its material - gun metal shines, plastic less, gloves hardly.
- Diegetic Scope: HDTP's rifle no longer draws its blue lens cap over the live scope picture.
- First-person body: no blob shadow under JC while it's on.
- Enhancements menu: a description too long for its box scrolls slowly to the end and starts over.
- Controller: D-pad snap-to works on the Achievements screen.
- Linux: starting in fullscreen no longer crashes when the window opens before the monitor is known (it opens
  windowed and switches to fullscreen a moment later).

### Mods folder, optional fixes, healing by body part (November 2)
- One folder for all your mods: on the launcher's Mods page, Mod Assets Folder... takes a folder holding each mod in
  its own folder (Lay D Denton, HDTP, New Vision, Helios, the Parallax Occlusion addon, Augmentique, Visible
  Attachments, GMDX: Augmented Edition and its Precipitation) and sets them all up.
- Optional Fixes (Fixes, on in Modern): misnamed mission flags
  (the Hong Kong ROM, the ship breach, the Vandenberg hazlab troops, Aimee's goal, the Karkian tank, Jock in Area 51),
  the ATM accepting any account number with the first account's PIN, EMP'd robots still attacked, bot screens
  staying open after the bot is destroyed, the laser left on in conversations, the riot prod breaking crates, people
  set alight dying "unconscious", barks under water, leather chair and couch colours, and more.
- Bigger changes as separate options (all off): Unconscious Bodies Can Die, Using Things Makes Noise,
  People Mind A Pointed Gun, Thermoptic Camo Fools Machines, Wounded Legs Slow People, Stronger Gunther And Simons,
  Long-Burning Flares.
- Diegetic Healing: with a medkit in hand, hold Alt (controller: LT) to bring up your body; move the little medkit
  over a part and click (A) to treat it - one medkit per use, without opening the inventory.
- Aug glows: each running augmentation glows in its own colour; vision augs shine from the eyes. Healing sends green
  leaves drifting up from the treated part.
- Third person: weapons stay the right way up while aiming and moving; leaning bends the body instead of sliding it.
- The rifle's diegetic scope follows the gun (recoil, sway, idle animations) and fits its eyepiece, HDTP's too.
- Storm water no longer tears open into flat triangles; coats swing more naturally; ragdolls can't pass through
  walls, doors or furniture.

### Upscaling, shadows, grass and trees (November 2)
- Upscaling (Settings > Enhancements > Display): Off, Native AA (temporal antialiasing at full resolution plus a light
  sharpen), and Quality / Balanced / Performance / Ultra Performance, which draw the world at 1/1.5, 1/1.7, 1/2 or
  1/3 the window's size, antialias it temporally and upscale it with AMD FidelityFX Super Resolution 1's
  edge-adaptive filter (EASU, ported to WGSL) and RCAS sharpening. The weapon, HUD and menus stay at full resolution.
- Real-time shadows reworked: shadow lights are chosen from all around you (not just ahead), lights on your own
  floor count most, a light already shown holds its place, and each fades in and out by its rank instead of popping.
  Every model casts from one light only (the one that lights it most), so people no longer drag three shadows. A
  shadow only takes away light the map's own lighting gave that spot, and overlapping lights share it out, so
  shadows are never darker than the light they block. People now receive shadows too (from other people and
  objects, and their own arms).
- The flashlight cone is a plain cone worked out in the shaders - a hot spot with a dim spill round it - with no
  shadow map (no acne on walls beside you, one less shadow pass).
- 3D Grass (Visuals: Off / Low / High): blades grow out of the lawns on the GPU, lit by the ground's own light map,
  swaying in travelling gusts and pushed aside by whoever walks through. Grass grows on the textures
  SecondRevision chose (the clean, light and dark green lawns and the natural grass).
- Grown Trees (Visuals): the parks' flat card trees and bushes are replaced with grown ones - space-colonisation
  branches, bark, clusters of leaves - four variants of each kind per map, at the original's size and place, lit by
  the map's lights, swaying in the wind and casting shadows. The original's collision is kept.
- Glowing Surfaces now also light their surroundings: augmentation canisters, bio cells, computer and ATM screens
  and keypads give off a small light of their own colour.
- With Glow And Bloom, coronas feed the bloom (a hot core at each lamp that the bloom spreads).
- Combat HUD (Interface): out of combat the crosshair and the status displays fade away; they return at once in a
  fight, when aiming down the sights or at an enemy, firing, hurt, badly hurt or changing weapons.
- With New Vision's textures on, SecondRevision's height maps for them (if installed) give their walls relief.
- Classic mode: detail textures (Display: the original renderers' close-up detail pass); zone reverb (the zones'
  own echo settings, when the modern reverb is off); zone ambient light on walls and people; animated water, wave
  and ice textures (WetTexture / WaveTexture drop simulations) instead of our swell and ripples on classic water.
- Fish swim the way they face again, and the crosshair turns green over birds and fish.

### Playtest fixes (November 1, night)
- Maps no longer pop in surface by surface: after a load the screen stays black with "LOADING" (game time and cut
  scenes held) until the new map's shaders have compiled, at most 6 seconds. The intro's first room appears whole.
- Cut scenes take no game input: the pad's flashlight, belt and weapon buttons did things during the intro. The
  intro's end now says "Press A, Menu or RT to continue" with a pad (the pad's own names) and "Press Enter or Fire
  to continue" otherwise, and follows whichever was used last.
- Bob Page's line over JC in the clone tank is no longer near silent: cut scene speech always plays through the
  player (Sound Occlusion muffled it behind the tank's glass).
- No more free copy of an item: whatever is in hand must still be in the inventory, so using up a stack from the
  inventory screen, or the training course taking your things away, also takes it out of your hand.
- Flashlight cone: the narrower haze beam inside it (a second, inner corona against far ground) is gone; the cone
  alone lights the way.
- Security bots stay near where a fight starts (within their home extent) instead of chasing across the island;
  pawns with bDefendHome keep every combat move at home, as the original does.
- Smarter AI: an enemy glimpsed and lost for a moment no longer restarts the pawn's reaction time, which could leave
  it staring at you without ever firing.
- Wounds sit on the body: they're placed where the shot's line meets the limb (it was the collision cylinder, up to
  a foot out).
- Coats: no cloth on seated people (it fought the chair); otherwise the coat runs on a softer spring in fixed small
  steps, trails the body's motion and settles back, with a soft limit instead of a hard stop and a light breeze -
  it flows instead of jerking.
- Ragdolls: knees and elbows are hinges that bend only the way they should, twists are tighter and the body is
  damped more, so limbs no longer twist into broken shapes; the shot carries the whole body, hits above the hips
  topple it away from the shooter and leg hits sweep the legs.
- Modern Water: the water's colour no longer uses the lightmap, whose per-surface blocks showed as blue-grey angular
  patches and seams; lamps still show in the reflection and as glints.
- Codes From Notes: a keypad used in the world shows "Tab: code from notes" (pad: Y) at the bottom of the screen;
  computer logins found in the notes are now offered the same way ("Tab: log in from notes" under the fields)
  instead of being filled in silently.
- Diegetic Scope: the sniper rifle stays in view and you look through its own scope - the lens sits in the
  eyepiece found on the model (original and HDTP), with a glass sheen, a cool tint and a colour fringe at its edge.

### Playtest fixes (November 1, evening)
- First-person body: it now stands back as far as the pose needs, so the cut chest is never in view, looking straight
  down or walking, running and aiming (those poses lean the chest forward); looking down you see your legs and feet.
- Weapons held by the rebuilt characters sit right again: their weapon triangle is turned against the original
  mesh's (JC and many NPCs held their guns upside down or sideways).
- Blinking slides the eyelid's picture up to the crease instead of stretching what's under it (Paul's eyeball
  stretched over his eye).
- Relief maps: parallax depth now matches each texture's size (a fixed depth was up to 36 units deep on large
  textures, so bricks slid and smeared); more layers.
- The flashlight no longer puts two orange balls in front of you: the aug glows only go on the parts of a body that
  are drawn (the first-person body has no head), and each body's glows only show with that body.
- Third person: aiming while looking down no longer drops the camera onto JC's head.
- Code: the player's shots, melee blows, lock-on and view-model posing / lighting are separate functions now
  (weapon_system 1,300 -> under 900 lines); text fields place the caret without building strings.

### Playtest fixes (November 1)
- Paul runs to you on the dock again: when re-planning his way he kept picking the nearest path point even when it
  was behind him, so he ran back and forth; and a straight line through the dock's row of BlockMonsters (which stop
  every pawn but the player) counted as clear, so he ran on the spot against it. Pawns now take the path point that
  leads toward where they're going, and never plan a straight line through a BlockMonsters.
- Water reflections no longer show trees and people through walls: a reflection camera switched off and on again
  lost the level's walls (they're culled on the GPU and Bevy only hands them to a camera once). The level's surfaces
  are now handed to every camera again whenever one comes on, and water reflects every frame.
- Stormy water: in rain the sea gets a long storm swell, rougher short waves and whitecaps, and the surface moves up
  to three and a half times as far; water sheets no longer open cracks where two of the map's water surfaces meet
  (seams are mended across all of them, and the dark backing sits deeper, under a storm's troughs).
- The open sea only goes where the map's edge is open water, and only on maps that have sea; the 3D skyline is only
  built from the map's own skyline photos (no stand-in towers elsewhere); maps that only see the sky through windows
  get no volumetric clouds.
- First-person body: stands further back and higher, keeps the chest, and is drawn from the outside only, so the
  opening at the cut is out of view when you look down.
- Aim down the sights now lines up on the original weapon models too (the barrel's tip is put under the crosshair);
  HDTP models keep their own alignment.
- Diegetic Scope (Settings > Enhancements, on in Modern): looking through a rifle scope is a live lens in front of
  the eye with a reticle and the scope's tube, the world still around it, instead of a black screen.
- Smoother animation: the first-person weapon eases into each new animation (a swing, a shot, a fidget) instead of
  snapping to it, and the rebuilt characters blend between moves over a quarter of a second.
- Rain no longer hitches when you walk into new ground or climb into a new height band: where drops land is worked
  out a few hundred spots at a time.
- A frame-time log for hitches: frames much slower than the running average are written to hitch.log with the
  slowest parts of that frame.
- The AI code: a pawn's shot is now a few plain functions (start, ammo, one traced slug), and door handling, orders
  and animal startles left the per-pawn loop.

### Remastered lighting (enhancements, on in Modern)
- Remastered Lighting: every wall and floor relit per pixel by its strongest lamps, with real falloff and shape,
  keeping the original's baked shadows.
- Relief Maps: normal and parallax maps - the Parallax Occlusion Mapping addon's (shipped with the release; only its
  normal / height maps are read, so it sits beside Helios and New Vision), generated ones for brick, stone, wood,
  tiles, grates and metal elsewhere.
- Material Highlights: metal, glass, tiles and polished floors catch the light.
- Glowing Surfaces: light fittings, screens and signs glow past white, with Bloom's halo.

### Faster
- About 30 to 47 fps on an RTX 3050 in Modern: real-time shadows render one view per light instead of six (and
  only for lights with someone in them), rain is built on the GPU, water reflections re-render every other frame.
- A CPU performance pass after a report of poor frame rates on a Xeon E5-2680 v4 with an RTX 2070. The GPU was never
  the limit: the game's main thread and Bevy's render thread were, so a CPU with slower cores dropped under 60 fps
  and VSync then halved it to 30. Measured at default settings on a Ryzen 5 3600 with VSync off: the game's main
  thread went from 13.9 ms of work a frame to 7.9 ms, Bevy's render thread from about 12 ms to 9.4 ms, and the frame
  from 13.8 ms (72 fps, already with the first two fixes below) to 10.2 ms (98 fps).
  - Every collision test (AI movement, sight lines, bullets, physics) now skips the cylinders and doors it can't
    reach before doing any real maths: the AI costs a third of what it did.
  - The ragdoll physics engine sleeps while no ragdoll is falling (it ran a full step every frame, Ragdolls on or off).
  - The open sea and the water surfaces were having their bounds recomputed every frame (1.6 million vertices),
    because their materials animate; level surfaces' bounds are now worked out once.
  - The sky room is drawn by the world camera (scaled out past the map) instead of a camera of its own: one whole
    camera's work less on the render thread.
  - The world camera's depth pre-pass only runs once Modern Water has been on (it's what reads it).
  - Fewer, better-used threads: on CPUs with many cores (workstation Xeons, Threadrippers, dual-socket boards) the
    worker pool stops at 10 threads instead of spreading each frame's small jobs over every core; a new setting
    (Settings > Enhancements > Display: CPU Worker Threads) overrides it. The render passes no longer flood the
    worker pool.
  - Bevy features the game doesn't use (its gamepad polling, animation and picking systems) are switched off.
  - The release build is optimised as one whole program.
- A GPU-driven drawing pass, so the processor hands the graphics card whole batches instead of one draw at a time.
  Modern with every option and mod on a Ryzen 5 3600 + RTX 3050: about 1,000 draw calls a frame down to about 250,
  and 24 ms (41 fps) down to about 14 ms (70 fps); Classic from about 12 ms (82 fps) to 11 ms (92 fps).
  - The world's surfaces and the rebuilt characters' materials are bindless: hundreds of textures share a few bind
    groups, so the GPU draws them in a handful of multi-draw calls (it was one draw call per texture, per camera).
  - The level's surfaces are culled by the GPU (it tests each one against the view before drawing) instead of on the
    main thread, and stay filed in the render thread's draw lists instead of being re-filed as the camera turns.
  - Rain falls on the GPU: each drop and splash is written once, and the shader moves it on the GPU's own clock.
    The CPU spawns a batch five times a second instead of moving 11,000 drops and uploading them every frame.
  - Water waves and panning textures run on the GPU's clock too; their materials no longer change every frame (each
    change made the render thread prepare the material again). Character lighting, the clouds and the skyline's
    beacons only touch their materials when something actually changed.
  - All the blob shadows under people are one mesh (they were fifty-odd separate transparent draws per camera).
  - Only the head of a rebuilt character carries its face shapes.
  - Avian no longer re-propagates every transform in the world before each physics step while no ragdoll is
    falling, and the first-person body takes its pose from JC's mirror body instead of sampling it a second time.

- A code-health and allocation pass. The game now uses mimalloc (a fast multi-threaded allocator) and profile
  runs count heap allocations per frame and per system. The game's own systems made about 6,000 allocations a frame
  with HDTP installed (a few hundred without); the worst offenders were real waste and are gone:
  - JC's body mesh was worked out every frame by the mirror and first-person bodies, each time copying HDTP's whole
    class-override list (thousands of entries) - about 5,400 allocations and a lot of copying a frame. It's now
    cached and looked up in place.
  - The NPC update rebuilt its snapshot of every pawn (names, alliances) every frame: it's now one buffer kept
    between frames and updated in place; IsA class checks, actor tags, the session (difficulty, skin) and lowercase
    name tests no longer make copies of strings.
  - Triggers, the HUD (it hashed formatted text of the whole item belt every frame), achievements, sound (held
    loops), dynamic lights, fire textures, conversations, impacts and blood no longer rebuild lists, hash sets or
    class defaults every frame; cross-system queues reuse their buffers.
  - Character animation looks up its clip only when the sequence changes (it built two or three strings per body
    per frame), and the weapon frame and coat cloth reuse their bone matrices and cached bone indices.
  - Poison and bleeding moved out of the NPC update into their own component and system (with tests), a first step
    in splitting up the NPC component.
  - The NPC update is broken up: it was one function of about 3,200 lines nested up to 16 levels deep; damage
    handling (with dying and being hurt apart), sight, hearing, first-frame setup and the Attacking, firing, Fleeing, Alerting and Burning states are
    now their own functions with explicit inputs (the update itself is about 1,500 lines, nested at most 8 deep).
    Line-of-sight and alliance checks are one shared `Senses` type instead of closures repeated in each part.
  - Unused code removed and compiler warnings down from 65 to 19.

### Launcher
- Mods page: a Parallax Maps row to point the game at the Parallax Occlusion Mapping addon's folder (turn it on or
  off like the other mods); without it the game looks in the release's relief folder and the game's Textures folder.

### Memory
- Spoken lines no longer pile up: every line heard stayed decoded for the rest of the session (hundreds of megabytes
  over a playthrough); speech now keeps to 128 MB, dropping the oldest lines.
- A map's textures are no longer kept in main memory once they're on the graphics card (tens of megabytes a map,
  far more with HD texture packs).

### Bodies (enhancements, on in Modern)
- Skeletal Characters: the Machine God team's rebuilt human bodies on a real skeleton, in the original skins, with
  blended moves and a face that lip-syncs, blinks and glances. JC's mirror and third-person body too.
- Ragdolls (Avian physics): the killed and knocked out go limp at once, pushed by the hit and carried by their run.
- Fluid Blood: droplets that fly and splatter, blood running down walls, pools spreading under the dead, bloodier gibs.
- Wounds on bodies, hurt poses (hanging arms, limps), coat cloth.
- First-person body (waist and legs, as Half-Life mods show it), diegetic healing (treat the limb you look at with a medkit; elsewhere it treats the head),
  aug glows on JC's body and a healing glow.
- JC dies as a ragdoll too. Helios repaints and Augmentique outfits (JC, Lay D, the people they dress) carry over to
  the rebuilt bodies; HDTP's people are off while Skeletal Characters is on, so JC keeps his rebuilt body.

### Fixed
- Linux (Wayland): full screen at start-up crashed with "Unable to get monitor" (GitHub #4); full screen is
  borderless on the first monitor there.
- Falling through the world after loading a save made while crouched (GitHub #2): the crouched position was lowered
  a second time on load, putting JC's feet under the floor.
- Black triangles all over the level with real-time shadows: surfaces outside a shadow light's cone read its
  shadow map anyway.
- Rebuilt characters that stood frozen, unlit and black (a body missed for one frame stayed unposed); they are now
  posed and lit whenever near.
- Black sunglasses on people who wear none (the hidden glasses slot was being lit).
- Coat tails whipping and flipping: they now ripple, damped and held close to the animation.
- Rain splashing under the dock roof when the wind slanted the drops in.
- JC's skeletal, ragdoll and first-person bodies missing with HDTP or Augmentique installed.
- Rebuilt characters never animated or lip-synced: every sequence lookup found the mesh's "All" sequence, looping
  moves froze after one pass, and the bodies were packed without their face shapes (Blender writes them as sparse
  glTF data, which the packer read as zeros). Faces now blink, glance and lip-sync; standing people's faces are
  re-sent to the GPU so the shapes show.
- Placed bodies (the one with the pepper spray) stood in a black T-pose: carcass meshes keep their own lying pose.
- First-person body: no more black outlines or the inside of the coat in view.
- Rebuilt characters darker than the originals (only 8 lights reached them; now 16 and an even fill for the rest).

### Removed
- Wet Bodies (everyone came out shiny and smooth).

## v0.10.0

Everything new since v0.9.0. Every change to the original game is still an option in *Settings > Enhancements*,
off unless you switch it on (or press Tab for Modern).

### Classic / Modern switch
- Press **Tab** in play to switch between the original look and a modern one, with a full-screen wipe. Modern turns
  on every enhancement at once; going back to Classic remembers your own Modern mix.
- The visual mods (HDTP, New Vision, Helios, Visible Attachments) switch with it, reloading the map in place.
- The switch is quicker, and shows the blue LOADING while it works.

### A modern look (enhancements)
- Real-time shadows from the map's own lights, ambient occlusion, soft shadows, bloom on lights and screens.
- Rain (with the Precipitation mod), with splashes, ripples, footstep splashes and rain kept out from under roofs.
- Storms: volumetric night clouds, lightning bolts that light the clouds, walls and sky, and thunder.
- Modern water: waves, reflections, depth and an open sea out to the horizon.
- Photographic city skyline, light halos, modern coronas, local fog, rolling mist and light shafts.
- Modern night vision and thermal views.
- Enhanced effects: particle impacts, explosions, muzzle smoke, sparks, barrel smoke, bullet holes and tracers.
- Helios texture packs (characters, items, world), with blink fixes for the repainted faces.

### Modern play (enhancements)
- Modern Aiming: aim sways inside the accuracy cone, the crosshair and gun follow it, and aim down the sights on
  guns. Untrained hands sway widely, masters hold steady, and standing still draws the aim in.
- Third person: free orbit or over-the-shoulder camera, with JC's full set of moves.
- Mantling, carried objects as cover, ladder grabbing.
- Elements: shocks spread through water, fire spreads between people, water puts it out.
- Smarter AI from GMDX: sharper guards, quick reflexes, suppressive fire, guards who call out and check lights
  going out and doors opening. Optional alert markers over their heads.
- Terminals in the world: computers, ATMs, security consoles and keypads are used on their own screens.
- Bullet penetration, melee arcs, thrown-object noise, body alarms, persistent debris, inertia and smooth leaning.
- GMDX perks with their effects, GMDX fixes and quality-of-life options.
- Better audio: smoother mixing, sound through walls, room reverb and a distant soundscape.

### Achievements
- Fifty achievements, with their own persona page and an unlock notice in play.

### Mods
- Augmentique outfits and weapon skins, Visible Attachments, GMDX: Augmented Edition, Helios and Precipitation join
  Lay D Denton, HDTP and New Vision.

### Controller and interface
- Dialogue hotkeys and keypad codes from notes on the pad. X grabs ladders.
- HUD Scale sizes the menus, inventory and computer screens too.
- V-Sync option, credits easter eggs, a searchable and grouped Enhancements list, Discord Rich Presence.
- The persona screens' nav bar scrolls when Outfits and Achievements are both there; Left / Right switch screens.

### Fixes
- Turrets fire again. NPCs open doors (locked ones too, as in the original) and wait for them.
- Datacubes add their text to your notes.
- Mirrors keep reflecting when you look straight down at them.
- Faster loading; vertex explosions on dense models fixed.
- The intro: the camera keeps the original framing (it ran at a 149-degree field of view from the street scene on),
  the clone tank is lit and drawn as in the original, the female clone holds still, and the intro ends with
  "Press Enter to continue" instead of hanging.
- Mesh lighting is one-sided as in the original, and special lights no longer light characters.
- Rain splashes no longer show through roofs. Water reflections stay put in third person.
- Music is quieter at low settings and steps back under speech.
- The intro no longer awards mission achievements.
