# Changelog

## Unreleased

### Faster
- About 30 to 47 fps on an RTX 3050 in Modern: real-time shadows render one view per light instead of six (and
  only for lights with someone in them), rain is built on the GPU, water reflections re-render every other frame.

### Bodies (enhancements, on in Modern)
- Skeletal Characters: the Machine God team's rebuilt human bodies on a real skeleton, in the original skins, with
  blended moves and a face that lip-syncs, blinks and glances. JC's mirror and third-person body too.
- Ragdolls (Avian physics): the killed and knocked out go limp at once, pushed by the hit and carried by their run.
- Fluid Blood: droplets that fly and splatter, blood running down walls, pools spreading under the dead, bloodier gibs.
- Wounds on bodies, hurt poses (hanging arms, limps), coat cloth, wet bodies in the rain.
- First-person body, diegetic healing (treat the limb you look at with a medkit; elsewhere it treats the head),
  aug glows on JC's body and a healing glow.

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
