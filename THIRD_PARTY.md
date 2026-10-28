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
The Lay D Denton Project belongs to its authors. DX RUST includes none of
its files: it reads them from the folder the player chooses.

## Rust crates
DX RUST's dependencies (Bevy, egui / eframe, gilrs, hidapi, cpal, rfd, serde and others) keep their own licences,
listed in each crate. `cargo about` or `cargo license` prints the full list.
