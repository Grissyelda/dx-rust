//! DDS files (texture override packs such as Helios, made for kentie's Direct3D 10 renderer): the header, the mip
//! chain as stored, and decoding to RGBA8 for formats the GPU path doesn't take as they are.
//!
//! Handled: DXT1 / DXT3 / DXT5 (FourCC or DX10 BC1-3), BC7 (DX10), and uncompressed 32-bit (A8R8G8B8 /
//! X8R8G8B8 / A8B8G8R8), 24-bit and 16-bit (R5G6B5, A1R5G5B5, A4R4G4B4) RGB.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Bc1,
    Bc2,
    Bc3,
    Bc7,
    /// Uncompressed: bytes per pixel and the R, G, B, A bit masks.
    Rgb { bpp: usize, masks: [u32; 4] },
}

pub struct Dds {
    pub width: u32,
    pub height: u32,
    pub format: Format,
    /// Mip 0 first, as stored.
    pub mips: Vec<Vec<u8>>,
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

pub fn level_size(format: Format, w: u32, h: u32) -> usize {
    let bw = w.div_ceil(4) as usize;
    let bh = h.div_ceil(4) as usize;
    match format {
        Format::Bc1 => bw * bh * 8,
        Format::Bc2 | Format::Bc3 | Format::Bc7 => bw * bh * 16,
        Format::Rgb { bpp, .. } => w as usize * h as usize * bpp,
    }
}

pub fn parse(b: &[u8]) -> Option<Dds> {
    if b.len() < 128 || &b[0..4] != b"DDS " {
        return None;
    }
    let height = u32le(b, 12);
    let width = u32le(b, 16);
    let mip_count = u32le(b, 28).max(1);
    let pf_flags = u32le(b, 80);
    let fourcc = &b[84..88];
    let mut off = 128;
    let format = if pf_flags & 0x4 != 0 {
        match fourcc {
            b"DXT1" => Format::Bc1,
            b"DXT2" | b"DXT3" => Format::Bc2,
            b"DXT4" | b"DXT5" => Format::Bc3,
            b"DX10" => {
                if b.len() < 148 {
                    return None;
                }
                let dxgi = u32le(b, 128);
                off = 148;
                match dxgi {
                    70..=72 => Format::Bc1,
                    73..=75 => Format::Bc2,
                    76..=78 => Format::Bc3,
                    97..=99 => Format::Bc7,
                    28 | 29 => Format::Rgb { bpp: 4, masks: [0xff, 0xff00, 0xff0000, 0xff000000] },
                    87 | 91 => Format::Rgb { bpp: 4, masks: [0xff0000, 0xff00, 0xff, 0xff000000] },
                    _ => return None,
                }
            }
            _ => return None,
        }
    } else if pf_flags & 0x40 != 0 {
        let bits = u32le(b, 88) as usize;
        let a = if pf_flags & 0x1 != 0 { u32le(b, 104) } else { 0 };
        Format::Rgb { bpp: bits / 8, masks: [u32le(b, 92), u32le(b, 96), u32le(b, 100), a] }
    } else {
        return None;
    };
    if let Format::Rgb { bpp, .. } = format {
        if !(2..=4).contains(&bpp) {
            return None;
        }
    }
    let mut mips = Vec::new();
    let (mut w, mut h) = (width.max(1), height.max(1));
    for _ in 0..mip_count {
        let n = level_size(format, w, h);
        if off + n > b.len() {
            break;
        }
        mips.push(b[off..off + n].to_vec());
        off += n;
        if w == 1 && h == 1 {
            break;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    if mips.is_empty() {
        return None;
    }
    Some(Dds { width, height, format, mips })
}

fn rgb565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 31) as u32;
    let g = ((c >> 5) & 63) as u32;
    let b = (c & 31) as u32;
    [((r * 527 + 23) >> 6) as u8, ((g * 259 + 33) >> 6) as u8, ((b * 527 + 23) >> 6) as u8]
}

/// The four colours of a BC1-style colour block (`four` = always 4-colour, as in BC2 / BC3).
fn color_block(blk: &[u8], four: bool) -> [[u8; 4]; 4] {
    let c0 = u16::from_le_bytes([blk[0], blk[1]]);
    let c1 = u16::from_le_bytes([blk[2], blk[3]]);
    let (a, b) = (rgb565(c0), rgb565(c1));
    let mix = |x: u8, y: u8, wx: u32, wy: u32| ((x as u32 * wx + y as u32 * wy) / (wx + wy)) as u8;
    let mut p = [[a[0], a[1], a[2], 255], [b[0], b[1], b[2], 255], [0; 4], [0; 4]];
    if four || c0 > c1 {
        p[2] = [mix(a[0], b[0], 2, 1), mix(a[1], b[1], 2, 1), mix(a[2], b[2], 2, 1), 255];
        p[3] = [mix(a[0], b[0], 1, 2), mix(a[1], b[1], 1, 2), mix(a[2], b[2], 1, 2), 255];
    } else {
        p[2] = [mix(a[0], b[0], 1, 1), mix(a[1], b[1], 1, 1), mix(a[2], b[2], 1, 1), 255];
        p[3] = [0, 0, 0, 0];
    }
    p
}

fn alpha_bc3(blk: &[u8]) -> [u8; 16] {
    let (a0, a1) = (blk[0] as u32, blk[1] as u32);
    let mut pal = [0u8; 8];
    pal[0] = a0 as u8;
    pal[1] = a1 as u8;
    if a0 > a1 {
        for i in 1..7u32 {
            pal[i as usize + 1] = (((7 - i) * a0 + i * a1) / 7) as u8;
        }
    } else {
        for i in 1..5u32 {
            pal[i as usize + 1] = (((5 - i) * a0 + i * a1) / 5) as u8;
        }
        pal[6] = 0;
        pal[7] = 255;
    }
    let bits = u64::from_le_bytes([blk[2], blk[3], blk[4], blk[5], blk[6], blk[7], 0, 0]);
    let mut out = [0u8; 16];
    for (k, o) in out.iter_mut().enumerate() {
        *o = pal[((bits >> (3 * k)) & 7) as usize];
    }
    out
}

/// One level to RGBA8.
pub fn decode_level(format: Format, data: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 4];
    match format {
        Format::Bc1 | Format::Bc2 | Format::Bc3 => {
            let bs = if format == Format::Bc1 { 8 } else { 16 };
            let bw = w.div_ceil(4);
            for (bi, blk) in data.chunks_exact(bs).enumerate() {
                let (bx, by) = ((bi % bw) * 4, (bi / bw) * 4);
                if by >= h {
                    break;
                }
                let (alpha, col) = match format {
                    Format::Bc1 => (None, blk),
                    Format::Bc2 => {
                        let mut a = [0u8; 16];
                        for (k, v) in a.iter_mut().enumerate() {
                            let nib = (blk[k / 2] >> ((k % 2) * 4)) & 15;
                            *v = nib * 17;
                        }
                        (Some(a), &blk[8..16])
                    }
                    _ => (Some(alpha_bc3(&blk[0..8])), &blk[8..16]),
                };
                let pal = color_block(col, format != Format::Bc1);
                let idx = u32::from_le_bytes([col[4], col[5], col[6], col[7]]);
                for k in 0..16 {
                    let (x, y) = (bx + k % 4, by + k / 4);
                    if x >= w || y >= h {
                        continue;
                    }
                    let mut p = pal[((idx >> (2 * k)) & 3) as usize];
                    if let Some(a) = alpha {
                        p[3] = a[k];
                    }
                    out[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&p);
                }
            }
        }
        Format::Bc7 => {
            let bw = w.div_ceil(4);
            for (bi, blk) in data.chunks_exact(16).enumerate() {
                let (bx, by) = ((bi % bw) * 4, (bi / bw) * 4);
                if by >= h {
                    break;
                }
                let mut px = [0u8; 64];
                bcdec_rs::bc7(blk, &mut px, 16);
                for k in 0..16 {
                    let (x, y) = (bx + k % 4, by + k / 4);
                    if x < w && y < h {
                        out[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&px[k * 4..k * 4 + 4]);
                    }
                }
            }
        }
        Format::Rgb { bpp, masks } => {
            let chan = |v: u32, m: u32| -> u8 {
                if m == 0 {
                    return 255;
                }
                let s = m.trailing_zeros();
                let bits = (m >> s).count_ones();
                let x = (v & m) >> s;
                ((x * 255 + ((1 << bits) - 1) / 2) / ((1 << bits) - 1)) as u8
            };
            for (i, px) in data.chunks_exact(bpp).enumerate().take(w * h) {
                let mut v = 0u32;
                for (k, b) in px.iter().enumerate() {
                    v |= (*b as u32) << (8 * k);
                }
                let o = i * 4;
                out[o] = chan(v, masks[0]);
                out[o + 1] = chan(v, masks[1]);
                out[o + 2] = chan(v, masks[2]);
                out[o + 3] = if masks[3] == 0 { 255 } else { chan(v, masks[3]) };
            }
        }
    }
    Some(out)
}

/// Every level to RGBA8.
pub fn decode_all(d: &Dds) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let (mut w, mut h) = (d.width.max(1), d.height.max(1));
    for m in &d.mips {
        match decode_level(d.format, m, w, h) {
            Some(px) => out.push(px),
            None => break,
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(w: u32, h: u32, mips: u32, fourcc: &[u8; 4]) -> Vec<u8> {
        let mut b = vec![0u8; 128];
        b[0..4].copy_from_slice(b"DDS ");
        b[4..8].copy_from_slice(&124u32.to_le_bytes());
        b[12..16].copy_from_slice(&h.to_le_bytes());
        b[16..20].copy_from_slice(&w.to_le_bytes());
        b[28..32].copy_from_slice(&mips.to_le_bytes());
        b[80..84].copy_from_slice(&4u32.to_le_bytes());
        b[84..88].copy_from_slice(fourcc);
        b
    }

    #[test]
    fn dxt5_solid_block() {
        let mut b = header(4, 4, 1, b"DXT5");
        // Alpha 128 everywhere, colour red (0xF800) everywhere.
        b.extend_from_slice(&[128, 128, 0, 0, 0, 0, 0, 0]);
        b.extend_from_slice(&[0x00, 0xF8, 0x00, 0xF8, 0, 0, 0, 0]);
        let d = parse(&b).unwrap();
        assert_eq!(d.format, Format::Bc3);
        let px = decode_all(&d);
        assert_eq!(&px[0][0..4], &[255, 0, 0, 128]);
        assert_eq!(&px[0][60..64], &[255, 0, 0, 128]);
    }

    #[test]
    fn mip_chain_sizes() {
        let mut b = header(8, 8, 4, b"DXT1");
        b.extend(std::iter::repeat_n(0u8, 32 + 8 + 8 + 8));
        let d = parse(&b).unwrap();
        assert_eq!(d.mips.len(), 4);
        assert_eq!(d.mips[0].len(), 32);
    }
}
