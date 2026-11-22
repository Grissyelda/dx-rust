# bevy_pbr 0.19.1, modified for DX RUST

This is bevy_pbr 0.19.1 (MIT / Apache-2.0, see LICENSE-MIT and LICENSE-APACHE) with one change:

- `src/lightmap/mod.rs`: entities that use the same lightmap image share one slot (`RenderLightmaps::by_image`,
  reference-counted; the slot is freed when its last user goes). Upstream gives every lightmapped entity a slot of
  its own, so a level lit from a single atlas filled one slab per four entities: a lightmap bind group per slab in
  every render phase, rebuilt every frame, and lightmapped meshes in different slabs could not share a draw.

Used through `[patch.crates-io]` in the workspace Cargo.toml.
