//! Procedural vegetation, engine-neutral (shared with the second game): plain data in, plain data out.
//!
//! - [`grass`]: a grass field laid over ground triangles as small tiles (height plane, coverage mask, light-map
//!   coordinates), the tiles near the eye packed each frame for a GPU shader that grows the blades
//!   ([`GRASS_WGSL`], Bevy material conventions) - no blade is stored.
//! - [`tree`]: trees and bushes grown from a species description (branching by space colonisation, tapering bark
//!   tubes, leaf cards), several variants per kind, as meshes.
//!
//! Coordinates are Y-up (Bevy's), in metres.

pub mod grass;
pub mod tree;

/// The grass blades' WGSL (vertex + fragment), for a Bevy `Material` binding the packed tile data at 0, the
/// light map at 1 / 2 and [`grass::GrassParams`] at 3 (see the shader's header).
pub const GRASS_WGSL: &str = include_str!("grass.wgsl");

/// A small stable hash (lowbias32), the same one the shaders use.
pub fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

/// A random number 0..1 from a hash state (advances it).
pub fn rand(s: &mut u32) -> f32 {
    *s = hash(*s);
    (*s & 0xFF_FFFF) as f32 / 16_777_216.0
}
