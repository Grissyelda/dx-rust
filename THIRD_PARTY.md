# Third-party notices

## SurrealEngine
Some DX RUST code was written in Rust with SurrealEngine (https://github.com/dpjudas/SurrealEngine) as a reference.
That covers BSP box tracing, fire textures, PHYS_Swimming and parts of the movement physics, the sound mixer
volumes, and corona and fog-ball lighting. These are altered versions, not the original software. The files
concerned say so in their headers.

SurrealEngine is under the zlib licence:

    SurrealEngine
    Copyright (c) 2021-2026 Magnus Norddahl, Lupert Everett and contributors

    This software is provided 'as-is', without any express or implied warranty. In no event will the authors be
    held liable for any damages arising from the use of this software.

    Permission is granted to anyone to use this software for any purpose, including commercial applications, and
    to alter it and redistribute it freely, subject to the following restrictions:

    1. The origin of this software must not be misrepresented; you must not claim that you wrote the original
       software. If you use this software in a product, an acknowledgment in the product documentation would be
       appreciated but is not required.
    2. Altered source versions must be plainly marked as such, and must not be misrepresented as being the
       original software.
    3. This notice may not be removed or altered from any source distribution.

## Deus Ex
Deus Ex and all of its content belong to their owners. DX RUST includes none of it: it reads the game files from
the player's own install at run time.

## Lay D Denton Project
The Lay D Denton Project belongs to its authors and is supported with their permission. DX RUST includes none of
its files: it reads them from the folder the player chooses.

## Other supported mods
HDTP, New Vision, Visible Attachments, Augmentique and GMDX: Augmented Edition belong to their authors. DX RUST
includes none of their files: it reads them from the folders the player chooses. Where a mod's behaviour is
supported, it was rewritten in Rust from the mod's source:
- GMDX AE's perks (Perk / PerkSystem and their effects), viewmodel inertia, the Dragon's Tooth energy cost, and
  the bugfixes and quality-of-life options listed in Settings > Enhancements;
- Augmentique's outfit, NPC outfit and weapon skin rules.

The only exception is Augmentique's outfit lists, which exist only as script code in the mod's own package. A small
interpreter reads them from the player's copy when the game starts, as data.

## Machine God team: rebuilt human bodies and rig
The *Skeletal Characters* enhancement uses the Machine God team's rebuilt human bodies, their Rigify rig and its
animations (`assets/rig`, baked from their Blender file by `tools/rig`). The team allows anyone to use the rig and
bodies as they please, with credit. Credit to the Machine God team.

These files contain no Deus Ex data. The original meshes and their vertex animations stay in the player's install;
the skins are read from the player's own game files at run time.

## Joe Wintergreen's lipsync mod
The *Smooth Lip Sync* enhancement uses the timings from Joe Wintergreen's lipsync mod for Deus Ex:
- mouth shapes blend over 0.36 s;
- the mouth closes over 0.3 s;
- blinks are checked every 0.5 s and play at a fifth of the speed.

It was rewritten in Rust; none of the mod's files are included. Credit to Joe Wintergreen (@joewintergreen).

## Rust crates
DX RUST's dependencies (Bevy, egui / eframe, gilrs, hidapi, cpal, rfd, serde and others) keep their own licences,
listed in each crate. `cargo about` or `cargo license` prints the full list.
