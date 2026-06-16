//! Bit-exact Rust port of `GPU3D::SoftRenderer::TextureLookup`
//! (melonDS `src/GPU3D_Soft.cpp`). Pure function of the texture parameters
//! and the two flat VRAM arrays the C++ reads via ReadVRAMFlat_Texture /
//! ReadVRAMFlat_TexPal:
//!   - `tex`: VRAMFlat_Texture, 512 KiB, addresses masked & 0x7FFFF
//!   - `pal`: VRAMFlat_TexPal,  128 KiB, addresses masked & 0x1FFFF
//!
//! Returns (color: u16 BGR555, alpha: u8 0..=31), exactly as the C++ writes
//! through its `*color` / `*alpha` out-params.

pub struct Vram<'a> {
    pub tex: &'a [u8],
    pub pal: &'a [u8],
}

impl Vram<'_> {
    #[inline]
    fn tex_u8(&self, addr: u32) -> u8 {
        self.tex[(addr & 0x7FFFF) as usize]
    }
    #[inline]
    fn tex_u16(&self, addr: u32) -> u16 {
        let i = (addr & 0x7FFFF) as usize;
        // C++ reads two linear bytes; real texture data never reaches the very
        // last byte, but mask the high byte too so Rust can't index OOB.
        let lo = self.tex[i] as u16;
        let hi = self.tex[(i + 1) & 0x7FFFF] as u16;
        lo | (hi << 8)
    }
    #[inline]
    fn pal_u16(&self, addr: u32) -> u16 {
        let i = (addr & 0x1FFFF) as usize;
        let lo = self.pal[i] as u16;
        let hi = self.pal[(i + 1) & 0x1FFFF] as u16;
        lo | (hi << 8)
    }
}

/// Port of TextureLookup. `s`/`t` are the raw s16 texcoords (pre `>>4`).
#[inline]
pub fn texture_lookup(vram: &Vram, texparam: u32, texpal: u32, s: i16, t: i16) -> (u16, u8) {
    let mut vramaddr = (texparam & 0xFFFF) << 3;

    let width: i32 = 8 << ((texparam >> 20) & 0x7);
    let height: i32 = 8 << ((texparam >> 23) & 0x7);

    let mut s: i32 = (s as i32) >> 4;
    let mut t: i32 = (t as i32) >> 4;

    // texture wrapping (S)
    if texparam & (1 << 16) != 0 {
        if texparam & (1 << 18) != 0 {
            if s & width != 0 {
                s = (width - 1) - (s & (width - 1));
            } else {
                s &= width - 1;
            }
        } else {
            s &= width - 1;
        }
    } else if s < 0 {
        s = 0;
    } else if s >= width {
        s = width - 1;
    }

    // texture wrapping (T)
    if texparam & (1 << 17) != 0 {
        if texparam & (1 << 19) != 0 {
            if t & height != 0 {
                t = (height - 1) - (t & (height - 1));
            } else {
                t &= height - 1;
            }
        } else {
            t &= height - 1;
        }
    } else if t < 0 {
        t = 0;
    } else if t >= height {
        t = height - 1;
    }

    let alpha0: u8 = if texparam & (1 << 29) != 0 { 0 } else { 31 };

    let mut color: u16 = 0;
    let mut alpha: u8 = 0;

    match (texparam >> 26) & 0x7 {
        1 => {
            // A3I5
            vramaddr = vramaddr.wrapping_add(((t * width) + s) as u32);
            let pixel = vram.tex_u8(vramaddr);
            let texpal = texpal << 4;
            color = vram.pal_u16(texpal + (((pixel & 0x1F) as u32) << 1));
            alpha = ((pixel >> 3) & 0x1C) + (pixel >> 6);
        }
        2 => {
            // 4-color
            vramaddr = vramaddr.wrapping_add((((t * width) + s) >> 2) as u32);
            let mut pixel = vram.tex_u8(vramaddr);
            pixel >>= (s & 0x3) << 1;
            pixel &= 0x3;
            let texpal = texpal << 3;
            color = vram.pal_u16(texpal + ((pixel as u32) << 1));
            alpha = if pixel == 0 { alpha0 } else { 31 };
        }
        3 => {
            // 16-color
            vramaddr = vramaddr.wrapping_add((((t * width) + s) >> 1) as u32);
            let mut pixel = vram.tex_u8(vramaddr);
            if s & 0x1 != 0 {
                pixel >>= 4;
            } else {
                pixel &= 0xF;
            }
            let texpal = texpal << 4;
            color = vram.pal_u16(texpal + ((pixel as u32) << 1));
            alpha = if pixel == 0 { alpha0 } else { 31 };
        }
        4 => {
            // 256-color
            vramaddr = vramaddr.wrapping_add(((t * width) + s) as u32);
            let pixel = vram.tex_u8(vramaddr);
            let texpal = texpal << 4;
            color = vram.pal_u16(texpal + ((pixel as u32) << 1));
            alpha = if pixel == 0 { alpha0 } else { 31 };
        }
        5 => {
            // compressed
            vramaddr = vramaddr.wrapping_add(((t & 0x3FC) * (width >> 2)) as u32);
            vramaddr = vramaddr.wrapping_add((s & 0x3FC) as u32);
            vramaddr = vramaddr.wrapping_add((t & 0x3) as u32);
            vramaddr &= 0x7FFFF; // wraps around after slot 3

            let mut slot1addr = 0x20000 + ((vramaddr & 0x1FFFC) >> 1);
            if vramaddr >= 0x40000 {
                slot1addr += 0x10000;
            }

            let val: u8 = if (0x20000..0x40000).contains(&vramaddr) {
                // reading slot 1 for texels should always read 0
                0
            } else {
                let v = vram.tex_u8(vramaddr);
                v >> (2 * (s & 0x3))
            };

            let palinfo = vram.tex_u16(slot1addr);
            let paloffset = ((palinfo as u32) & 0x3FFF) << 2;
            let texpal = texpal << 4;

            match val & 0x3 {
                0 => {
                    color = vram.pal_u16(texpal + paloffset);
                    alpha = 31;
                }
                1 => {
                    color = vram.pal_u16(texpal + paloffset + 2);
                    alpha = 31;
                }
                2 => {
                    if (palinfo >> 14) == 1 {
                        color = blend_compressed(vram, texpal, paloffset, 1, 1, 1);
                    } else if (palinfo >> 14) == 3 {
                        color = blend_compressed(vram, texpal, paloffset, 5, 3, 3);
                    } else {
                        color = vram.pal_u16(texpal + paloffset + 4);
                    }
                    alpha = 31;
                }
                3 => {
                    if (palinfo >> 14) == 2 {
                        color = vram.pal_u16(texpal + paloffset + 6);
                        alpha = 31;
                    } else if (palinfo >> 14) == 3 {
                        color = blend_compressed(vram, texpal, paloffset, 3, 5, 3);
                        alpha = 31;
                    } else {
                        color = 0;
                        alpha = 0;
                    }
                }
                _ => unreachable!(),
            }
        }
        6 => {
            // A5I3
            vramaddr = vramaddr.wrapping_add(((t * width) + s) as u32);
            let pixel = vram.tex_u8(vramaddr);
            let texpal = texpal << 4;
            color = vram.pal_u16(texpal + (((pixel & 0x7) as u32) << 1));
            alpha = pixel >> 3;
        }
        7 => {
            // direct color
            vramaddr = vramaddr.wrapping_add((((t * width) + s) << 1) as u32);
            color = vram.tex_u16(vramaddr);
            alpha = if color & 0x8000 != 0 { 31 } else { 0 };
        }
        _ => {} // format 0: untextured; C++ leaves color/alpha untouched
    }

    (color, alpha)
}

/// The compressed-format interpolation: (c0*wa + c1*wb) >> shift, per channel,
/// where shift is log2(wa+wb). The three call sites are (1,1)>>1, (5,3)>>3,
/// (3,5)>>3.
#[inline]
fn blend_compressed(vram: &Vram, texpal: u32, paloffset: u32, wa: u32, wb: u32, shift: u32) -> u16 {
    let color0 = vram.pal_u16(texpal + paloffset) as u32;
    let color1 = vram.pal_u16(texpal + paloffset + 2) as u32;

    let r0 = color0 & 0x001F;
    let g0 = color0 & 0x03E0;
    let b0 = color0 & 0x7C00;
    let r1 = color1 & 0x001F;
    let g1 = color1 & 0x03E0;
    let b1 = color1 & 0x7C00;

    let r = (r0 * wa + r1 * wb) >> shift;
    let g = ((g0 * wa + g1 * wb) >> shift) & 0x03E0;
    let b = ((b0 * wa + b1 * wb) >> shift) & 0x7C00;

    (r | g | b) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Independent reference transcription of GPU3D_Soft.cpp TextureLookup,
    // written separately from the production `texture_lookup` (different
    // structure, explicit u64 wrapping helpers) so a differential fuzz can
    // catch transcription mistakes in either. The authoritative oracle remains
    // the end-to-end hash gate against the real C++ on actual games; this fuzz
    // additionally covers format/size/wrap/palette combinations that a given
    // test ROM may never exercise.
    // -----------------------------------------------------------------------
    struct RefVram<'a> {
        tex: &'a [u8],
        pal: &'a [u8],
    }
    impl RefVram<'_> {
        fn t8(&self, a: u32) -> u32 {
            self.tex[(a & 0x7FFFF) as usize] as u32
        }
        fn t16(&self, a: u32) -> u32 {
            let i = (a & 0x7FFFF) as usize;
            self.tex[i] as u32 | ((self.tex[(i + 1) & 0x7FFFF] as u32) << 8)
        }
        fn p16(&self, a: u32) -> u32 {
            let i = (a & 0x1FFFF) as usize;
            self.pal[i] as u32 | ((self.pal[(i + 1) & 0x1FFFF] as u32) << 8)
        }
    }

    fn reference(rv: &RefVram, texparam: u32, texpal: u32, s_in: i16, t_in: i16) -> (u16, u8) {
        let mut vramaddr = (texparam & 0xFFFF).wrapping_shl(3);
        let width = 8i64 << ((texparam >> 20) & 0x7);
        let height = 8i64 << ((texparam >> 23) & 0x7);
        let mut s = (s_in as i64) >> 4;
        let mut t = (t_in as i64) >> 4;

        if texparam & (1 << 16) != 0 {
            if texparam & (1 << 18) != 0 {
                s = if s & width != 0 { (width - 1) - (s & (width - 1)) } else { s & (width - 1) };
            } else {
                s &= width - 1;
            }
        } else {
            s = s.clamp(0, width - 1);
        }
        if texparam & (1 << 17) != 0 {
            if texparam & (1 << 19) != 0 {
                t = if t & height != 0 { (height - 1) - (t & (height - 1)) } else { t & (height - 1) };
            } else {
                t &= height - 1;
            }
        } else {
            t = t.clamp(0, height - 1);
        }

        let alpha0: u8 = if texparam & (1 << 29) != 0 { 0 } else { 31 };
        let fmt = (texparam >> 26) & 0x7;
        let s = s as i64;
        let t = t as i64;
        let w = width;
        let add = |base: u32, off: i64| -> u32 { (base as i64).wrapping_add(off) as u32 };

        let mut color = 0u16;
        let mut alpha = 0u8;
        match fmt {
            1 => {
                vramaddr = add(vramaddr, t * w + s);
                let px = rv.t8(vramaddr);
                let tp = texpal << 4;
                color = rv.p16(tp + ((px & 0x1F) << 1)) as u16;
                alpha = (((px >> 3) & 0x1C) + (px >> 6)) as u8;
            }
            2 => {
                vramaddr = add(vramaddr, (t * w + s) >> 2);
                let mut px = rv.t8(vramaddr);
                px >>= (s as u32 & 0x3) << 1;
                px &= 0x3;
                let tp = texpal << 3;
                color = rv.p16(tp + (px << 1)) as u16;
                alpha = if px == 0 { alpha0 } else { 31 };
            }
            3 => {
                vramaddr = add(vramaddr, (t * w + s) >> 1);
                let mut px = rv.t8(vramaddr);
                if s & 1 != 0 { px >>= 4 } else { px &= 0xF }
                let tp = texpal << 4;
                color = rv.p16(tp + (px << 1)) as u16;
                alpha = if px == 0 { alpha0 } else { 31 };
            }
            4 => {
                vramaddr = add(vramaddr, t * w + s);
                let px = rv.t8(vramaddr);
                let tp = texpal << 4;
                color = rv.p16(tp + (px << 1)) as u16;
                alpha = if px == 0 { alpha0 } else { 31 };
            }
            5 => {
                vramaddr = add(vramaddr, (t & 0x3FC) * (w >> 2));
                vramaddr = add(vramaddr, s & 0x3FC);
                vramaddr = add(vramaddr, t & 0x3);
                vramaddr &= 0x7FFFF;
                let mut slot1 = 0x20000 + ((vramaddr & 0x1FFFC) >> 1);
                if vramaddr >= 0x40000 {
                    slot1 += 0x10000;
                }
                let val = if (0x20000..0x40000).contains(&vramaddr) {
                    0
                } else {
                    rv.t8(vramaddr) >> (2 * (s as u32 & 0x3))
                };
                let palinfo = rv.t16(slot1);
                let paloff = (palinfo & 0x3FFF) << 2;
                let tp = texpal << 4;
                let mix = |wa: u32, wb: u32, sh: u32| -> u16 {
                    let c0 = rv.p16(tp + paloff);
                    let c1 = rv.p16(tp + paloff + 2);
                    let r = ((c0 & 0x1F) * wa + (c1 & 0x1F) * wb) >> sh;
                    let g = (((c0 & 0x3E0) * wa + (c1 & 0x3E0) * wb) >> sh) & 0x3E0;
                    let b = (((c0 & 0x7C00) * wa + (c1 & 0x7C00) * wb) >> sh) & 0x7C00;
                    (r | g | b) as u16
                };
                match val & 0x3 {
                    0 => { color = rv.p16(tp + paloff) as u16; alpha = 31; }
                    1 => { color = rv.p16(tp + paloff + 2) as u16; alpha = 31; }
                    2 => {
                        color = match palinfo >> 14 {
                            1 => mix(1, 1, 1),
                            3 => mix(5, 3, 3),
                            _ => rv.p16(tp + paloff + 4) as u16,
                        };
                        alpha = 31;
                    }
                    3 => match palinfo >> 14 {
                        2 => { color = rv.p16(tp + paloff + 6) as u16; alpha = 31; }
                        3 => { color = mix(3, 5, 3); alpha = 31; }
                        _ => { color = 0; alpha = 0; }
                    },
                    _ => unreachable!(),
                }
            }
            6 => {
                vramaddr = add(vramaddr, t * w + s);
                let px = rv.t8(vramaddr);
                let tp = texpal << 4;
                color = rv.p16(tp + ((px & 0x7) << 1)) as u16;
                alpha = (px >> 3) as u8;
            }
            7 => {
                vramaddr = add(vramaddr, (t * w + s) << 1);
                color = rv.t16(vramaddr) as u16;
                alpha = if color & 0x8000 != 0 { 31 } else { 0 };
            }
            _ => {}
        }
        (color, alpha)
    }

    fn xorshift(s: &mut u64) -> u64 {
        let mut x = *s;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *s = x;
        x
    }

    #[test]
    fn fuzz_matches_independent_reference() {
        // Random VRAM contents, then sweep random texture parameters across all
        // formats / sizes / wrap modes / palettes / texcoords.
        let mut seed = 0x9E3779B97F4A7C15u64;
        let mut tex = vec![0u8; 512 * 1024];
        let mut pal = vec![0u8; 128 * 1024];
        for b in tex.iter_mut() {
            *b = xorshift(&mut seed) as u8;
        }
        for b in pal.iter_mut() {
            *b = xorshift(&mut seed) as u8;
        }
        let prod = Vram { tex: &tex, pal: &pal };
        let refv = RefVram { tex: &tex, pal: &pal };

        let cases = 3_000_000u64;
        for _ in 0..cases {
            let r = xorshift(&mut seed);
            let format = ((r >> 0) & 0x7) as u32;
            let wlog = ((r >> 3) & 0x7) as u32;
            let hlog = ((r >> 6) & 0x7) as u32;
            // wrap/flip bits 16..19, color0-transparent bit 29, offset bits 0..15
            let wrapbits = ((r >> 9) & 0xF) as u32; // -> bits 16..19
            let c0 = ((r >> 13) & 0x1) as u32; // -> bit 29
            let offset = (xorshift(&mut seed) & 0xFFFF) as u32; // bits 0..15
            let texparam = offset
                | (wrapbits << 16)
                | (format << 26)
                | (wlog << 20)
                | (hlog << 23)
                | (c0 << 29);
            let texpal = (xorshift(&mut seed) & 0x1FFF) as u32;
            let s = xorshift(&mut seed) as i16;
            let t = xorshift(&mut seed) as i16;

            let got = texture_lookup(&prod, texparam, texpal, s, t);
            let want = reference(&refv, texparam, texpal, s, t);
            assert_eq!(
                got, want,
                "texparam={texparam:08x} texpal={texpal:x} s={s} t={t}"
            );
        }
    }

    fn vram(tex: Vec<u8>, pal: Vec<u8>) -> (Vec<u8>, Vec<u8>) {
        let mut t = vec![0u8; 512 * 1024];
        let mut p = vec![0u8; 128 * 1024];
        t[..tex.len()].copy_from_slice(&tex);
        p[..pal.len()].copy_from_slice(&pal);
        (t, p)
    }

    // texparam fields: bits 26..28 = format, 20..22 = width log, 23..25 = height
    // log, bit 29 = color0 transparent, bits 0..15 = VRAM offset (>>3).
    fn texparam(format: u32, wlog: u32, hlog: u32) -> u32 {
        (format << 26) | (wlog << 20) | (hlog << 23)
    }

    #[test]
    fn format0_untextured_returns_zero() {
        let (t, p) = vram(vec![], vec![]);
        let v = Vram { tex: &t, pal: &p };
        assert_eq!(texture_lookup(&v, texparam(0, 0, 0), 0, 0, 0), (0, 0));
    }

    #[test]
    fn direct_color_reads_bgr555_and_alpha_bit() {
        // format 7, 8x8, texel (0,0) at vramaddr 0. Put 0x8123 there.
        let (t, p) = vram(vec![0x23, 0x81], vec![]);
        let v = Vram { tex: &t, pal: &p };
        let (color, alpha) = texture_lookup(&v, texparam(7, 0, 0), 0, 0, 0);
        assert_eq!(color, 0x8123);
        assert_eq!(alpha, 31, "high bit set -> opaque");

        // Clear the high bit -> transparent.
        let (t2, p2) = vram(vec![0x23, 0x01], vec![]);
        let v2 = Vram { tex: &t2, pal: &p2 };
        let (_, alpha2) = texture_lookup(&v2, texparam(7, 0, 0), 0, 0, 0);
        assert_eq!(alpha2, 0);
    }

    #[test]
    fn palette256_indexes_palette() {
        // format 4, texel 0 = palette index 5; palette[5] = 0x1234.
        let mut tex = vec![0u8; 8];
        tex[0] = 5;
        let mut pal = vec![0u8; 32];
        pal[10] = 0x34; // entry 5 -> byte offset 5*2 = 10
        pal[11] = 0x12;
        let (t, p) = vram(tex, pal);
        let v = Vram { tex: &t, pal: &p };
        // texpal=0 so palette base 0.
        let (color, alpha) = texture_lookup(&v, texparam(4, 0, 0), 0, 0, 0);
        assert_eq!(color, 0x1234);
        assert_eq!(alpha, 31, "nonzero index -> opaque");
    }

    #[test]
    fn palette256_index0_uses_alpha0_flag() {
        let tex = vec![0u8; 8]; // index 0
        let pal = vec![0u8; 32];
        let (t, p) = vram(tex, pal);
        let v = Vram { tex: &t, pal: &p };
        // bit29 clear -> alpha0 = 31 (opaque)
        let (_, a_opaque) = texture_lookup(&v, texparam(4, 0, 0), 0, 0, 0);
        assert_eq!(a_opaque, 31);
        // bit29 set -> alpha0 = 0 (transparent)
        let (_, a_trans) = texture_lookup(&v, texparam(4, 0, 0) | (1 << 29), 0, 0, 0);
        assert_eq!(a_trans, 0);
    }

    #[test]
    fn clamp_wrapping_keeps_s_in_range() {
        // format 4, 8x8, no repeat (clamp). s far beyond width clamps to 7.
        let mut tex = vec![0u8; 64];
        tex[7] = 9; // texel (7,0)
        let mut pal = vec![0u8; 64];
        pal[18] = 0xCD; // entry 9 -> 18
        pal[19] = 0xAB;
        let (t, p) = vram(tex, pal);
        let v = Vram { tex: &t, pal: &p };
        // s = 100 << 4 (pre >>4), clamps to width-1 = 7.
        let (color, _) = texture_lookup(&v, texparam(4, 0, 0), 0, 100 << 4, 0);
        assert_eq!(color, 0xABCD);
    }
}
