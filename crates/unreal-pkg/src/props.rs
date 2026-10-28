//! Property list decoding for Unreal Engine 1 (package version 68).
//!
//! Every serialized UObject starts with a property list: a sequence of tagged values
//! terminated by the name "None". Actors placed in a map store only the properties that
//! differ from their class defaults, so this is how we read where each actor is and how
//! it is configured.
//!
//! Tag layout (version 68):
//!   name        compact index into the name table ("None" ends the list)
//!   info byte   bits 0-3 type, bits 4-6 size class, bit 7 array flag
//!   [struct]    if type == Struct: struct name (compact index)
//!   size        by size class: 0=1, 1=2, 2=4, 3=12, 4=16, 5=u8 follows, 6=u16 follows, 7=u32 follows
//!   [array idx] if array flag and type != Bool: compact-style index (see `array_index`)
//! Bool values live in the array flag bit, not in the payload.

use crate::{Package, PkgError, Reader, Result};

pub const T_BYTE: u8 = 1;
pub const T_INT: u8 = 2;
pub const T_BOOL: u8 = 3;
pub const T_FLOAT: u8 = 4;
pub const T_OBJECT: u8 = 5;
pub const T_NAME: u8 = 6;
pub const T_STRING: u8 = 7;
pub const T_CLASS: u8 = 8;
pub const T_ARRAY: u8 = 9;
pub const T_STRUCT: u8 = 10;
pub const T_VECTOR: u8 = 11;
pub const T_ROTATOR: u8 = 12;
pub const T_STR: u8 = 13;
pub const T_MAP: u8 = 14;
pub const T_FIXED_ARRAY: u8 = 15;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Byte(u8),
    Int(i32),
    Bool(bool),
    Float(f32),
    /// Object reference; resolve with `Package::object_name`.
    Object(i32),
    /// Object reference resolved to a full path (`Package.Group.Name`), used once values from several
    /// packages are merged (see `classes`).
    Path(String),
    /// `InventoryItem` struct with its object resolved: (item class path, count).
    Item(String, i32),
    /// `InitialAllianceInfo` struct resolved: (alliance name, level -1..1, permanent).
    Alliance(String, f32, bool),
    Name(String),
    Str(String),
    Vector([f32; 3]),
    Rotator([i32; 3]),
    /// A struct we do not decode specially: struct name plus raw bytes.
    Struct(String, Vec<u8>),
    /// Anything else, kept raw so nothing is silently lost.
    Raw(u8, Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct Property {
    pub name: String,
    pub index: u32,
    pub value: Value,
}

fn size_from_class(class: u8, r: &mut Reader) -> Result<usize> {
    Ok(match class {
        0 => 1,
        1 => 2,
        2 => 4,
        3 => 12,
        4 => 16,
        5 => r.u8()? as usize,
        6 => r.u16()? as usize,
        7 => r.u32()? as usize,
        _ => unreachable!(),
    })
}

/// Array index encoding used inside property tags: one byte if < 128, two bytes
/// (high bit 0x80 set, next bit clear) if < 16384, otherwise four bytes (top bits 0xC0).
fn array_index(r: &mut Reader) -> Result<u32> {
    let b = r.u8()? as u32;
    if b < 0x80 {
        Ok(b)
    } else if b & 0xC0 == 0x80 {
        let b2 = r.u8()? as u32;
        Ok(((b & 0x3F) << 8) | b2)
    } else {
        let b2 = r.u8()? as u32;
        let b3 = r.u8()? as u32;
        let b4 = r.u8()? as u32;
        Ok(((b & 0x3F) << 24) | (b2 << 16) | (b3 << 8) | b4)
    }
}

/// Object flag set on exports that carry a script state frame before their properties.
pub const RF_HAS_STACK: u32 = 0x0200_0000;

/// Skip the state-frame header that precedes the property list of objects with RF_HasStack:
/// Node and StateNode (compact indices), ProbeMask (u64), LatentAction (i32), and an Offset
/// compact index only when Node is non-zero.
pub fn skip_state_frame(r: &mut Reader) -> Result<()> {
    let node = r.compact_index()?;
    let _state_node = r.compact_index()?;
    r.take(8)?; // ProbeMask
    r.take(4)?; // LatentAction
    if node != 0 {
        r.compact_index()?; // Offset
    }
    Ok(())
}

/// Decode a property list starting at the reader's position. Stops at "None".
pub fn read_properties(pkg: &Package, r: &mut Reader) -> Result<Vec<Property>> {
    let mut out = Vec::new();
    loop {
        let name_idx = r.compact_index()?;
        let name = pkg
            .names
            .get(name_idx as usize)
            .ok_or_else(|| PkgError::Unsupported(format!("bad property name index {name_idx}")))?
            .clone();
        if name == "None" {
            break;
        }
        let info = r.u8()?;
        let ty = info & 0x0F;
        let size_class = (info >> 4) & 0x07;
        let is_array = info & 0x80 != 0;

        let struct_name = if ty == T_STRUCT {
            Some(pkg.names.get(r.compact_index()? as usize).cloned().unwrap_or_default())
        } else {
            None
        };
        let size = size_from_class(size_class, r)?;
        let index = if is_array && ty != T_BOOL { array_index(r)? } else { 0 };

        let payload = r.take(size)?;
        let mut pr = Reader::new(payload, 0);
        let value = match ty {
            T_BYTE => Value::Byte(pr.u8()?),
            T_INT => Value::Int(pr.i32()?),
            T_BOOL => Value::Bool(is_array),
            T_FLOAT => Value::Float(pr.f32()?),
            T_OBJECT | T_CLASS => Value::Object(pr.compact_index()?),
            T_NAME => {
                let i = pr.compact_index()?;
                Value::Name(pkg.names.get(i as usize).cloned().unwrap_or_default())
            }
            T_STRING | T_STR => Value::Str(pr.fstring()?),
            T_STRUCT => {
                let sn = struct_name.clone().unwrap_or_default();
                match sn.as_str() {
                    "Vector" if size == 12 => Value::Vector([pr.f32()?, pr.f32()?, pr.f32()?]),
                    "Rotator" if size == 12 => Value::Rotator([pr.i32()?, pr.i32()?, pr.i32()?]),
                    _ => Value::Struct(sn, payload.to_vec()),
                }
            }
            other => Value::Raw(other, payload.to_vec()),
        };
        out.push(Property { name, index, value });
    }
    Ok(out)
}

/// Convenience lookups over a decoded property list.
pub fn find<'a>(props: &'a [Property], name: &str) -> Option<&'a Value> {
    props.iter().find(|p| p.name.eq_ignore_ascii_case(name)).map(|p| &p.value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_index_forms() {
        assert_eq!(array_index(&mut Reader::new(&[0x05], 0)).unwrap(), 5);
        assert_eq!(array_index(&mut Reader::new(&[0x81, 0x00], 0)).unwrap(), 256);
    }
}
