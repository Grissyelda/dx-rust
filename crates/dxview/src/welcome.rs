//! The welcome window on the main menu (menus.rs MenuScreenWelcome): what's new for players of the original, what to
//! look out for, and the state of Modern's performance. Shown at start until "Don't Show Again" (setting
//! WelcomeSeen = VERSION; raise VERSION when the text gains something worth showing again).

pub const VERSION: f32 = 1.1;

pub fn due() -> bool {
    setting!("WelcomeSeen") < VERSION
}

pub const TITLE: &str = "Welcome to DX RUST";

pub const TEXT: &str = "\
DX RUST is Deus Ex rebuilt from the ground up on a modern engine, running on your own copy of the game. Every \
mission, conversation and ending is here, played as it was in 2000 - and everything new is optional.

CLASSIC OR MODERN
Press Tab in play to switch between the original look and the Modern one, with a wipe across the screen. Every \
new feature is its own switch in Settings > Enhancements, so you can build the mix you like; Classic is always one \
key away.

THINGS TO TRY
- Real-time shadows, remastered lighting with relief on the walls, glowing lamps and screens, light shafts and fog.
- Modern Water: reflections, waves, rain rings and an open sea out to the horizon.
- Rain and storms with lightning (with the Precipitation mod), clouds and a 3D skyline.
- Rebuilt people with skeletons: blended animation, faces that talk, coats that move, ragdolls, wounds and blood \
that runs down walls.
- Third person (H, J swaps the shoulder), mantling, sprinting with stamina, ladder grabbing, carried objects as \
cover.
- Modern Aiming with aim-down-sights, a live rifle scope, weapon inertia, bullets through thin wood.
- Terminals, ATMs and keypads used right in the world.
- Smarter AI with optional alert markers, body alarms, guards who hear thrown objects.
- Item rotation and Pack in the inventory, codes from your notes, dialogue hotkeys.
- Fifty achievements, kept per character, with their own screen on this menu.
- Full controller support with button prompts, an aug wheel, and DualSense light bar, triggers and haptics.
- A Field of View slider and a frame rate cap (Settings > Enhancements > Display).
- Mods: Lay D Denton, HDTP, New Vision, Helios, GMDX's perks and fixes, Augmentique outfits and Visible \
Attachments, all from the launcher's Mods page.

MULTIPLAYER (NEW)
Multiplayer on this menu: host a room and share its code, or join one. In co-op the host plays the story and \
everyone shares the fight; each player keeps their own character, inventory and augmentations. Everyone marks \
Ready before the host starts. Press Y to chat. Classic rooms are played without mods, Modern rooms with the \
host's. The long room code works even when the online services are down (see the README). Deathmatch works but \
still needs work.

THINGS TO KNOW
- Each new character keeps its own saves, autosaves and achievements; the Load screen groups them by character.
- The game autosaves when you arrive in a map and when you complete a goal, but save often: this is a young \
project, and a bug can still get in the way of a mission.
- Found one? Report it on the project's GitHub page - a screenshot and your settings.txt help a great deal.

PERFORMANCE
Modern with every option on is still a work in progress and asks a lot of the processor. If the frame rate drops, \
turn off the most expensive options first - Light Shafts and Modern Water - or try Upscaling, a frame rate cap, or \
Classic. Classic runs well on most machines.

Thanks for playing.";
