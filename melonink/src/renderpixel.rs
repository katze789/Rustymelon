//! Bit-exact Rust port of `GPU3D::SoftRenderer::RenderPixel` (melonDS
//! `src/GPU3D_Soft.cpp`). Pure function: given the polygon attributes, the
//! interpolated vertex colour, and the texcoords, it returns the final shaded
//! pixel as `r | g<<8 | b<<16 | a<<24` (each 6-bit, a is 5-bit alpha 0..=31).
//!
//! It calls the already-ported `texture_lookup` internally, so the C++ caller
//! crosses the language boundary once per pixel instead of twice.
//!
//! Inputs mirror the C++ exactly:
//!   poly_attr   = polygon->Attr
//!   tex_param   = polygon->TexParam
//!   tex_palette = polygon->TexPalette
//!   disp_cnt    = gpu.GPU3D.RenderDispCnt
//!   toon_table  = gpu.GPU3D.RenderToonTable (32 × u16)

use crate::texture::{texture_lookup, Vram};

#[inline]
pub fn render_pixel(
    vram: &Vram,
    poly_attr: u32,
    tex_param: u32,
    tex_palette: u32,
    disp_cnt: u32,
    toon_table: &[u16; 32],
    mut vr: u8,
    mut vg: u8,
    mut vb: u8,
    s: i16,
    t: i16,
) -> u32 {
    let r: u8;
    let g: u8;
    let b: u8;
    let a: u8;

    let blendmode = (poly_attr >> 4) & 0x3;
    let polyalpha = ((poly_attr >> 16) & 0x1F) as u8;
    let wireframe = polyalpha == 0;

    if blendmode == 2 {
        if disp_cnt & (1 << 1) != 0 {
            // highlight mode: all vertex colour components set to the red one
            vg = vr;
            vb = vr;
        } else {
            // toon mode: vertex colour replaced by toon colour
            let tooncolor = toon_table[(vr >> 1) as usize];
            vr = ((tooncolor << 1) & 0x3E) as u8;
            if vr != 0 {
                vr += 1;
            }
            vg = ((tooncolor >> 4) & 0x3E) as u8;
            if vg != 0 {
                vg += 1;
            }
            vb = ((tooncolor >> 9) & 0x3E) as u8;
            if vb != 0 {
                vb += 1;
            }
        }
    }

    if (disp_cnt & (1 << 0) != 0) && (((tex_param >> 26) & 0x7) != 0) {
        let (tcolor, talpha) = texture_lookup(vram, tex_param, tex_palette, s, t);

        let mut tr = ((tcolor << 1) & 0x3E) as u8;
        if tr != 0 {
            tr += 1;
        }
        let mut tg = ((tcolor >> 4) & 0x3E) as u8;
        if tg != 0 {
            tg += 1;
        }
        let mut tb = ((tcolor >> 9) & 0x3E) as u8;
        if tb != 0 {
            tb += 1;
        }

        if blendmode & 0x1 != 0 {
            // decal
            if talpha == 0 {
                r = vr;
                g = vg;
                b = vb;
            } else if talpha == 31 {
                r = tr;
                g = tg;
                b = tb;
            } else {
                let ta = talpha as u32;
                r = (((tr as u32 * ta) + (vr as u32 * (31 - ta))) >> 5) as u8;
                g = (((tg as u32 * ta) + (vg as u32 * (31 - ta))) >> 5) as u8;
                b = (((tb as u32 * ta) + (vb as u32 * (31 - ta))) >> 5) as u8;
            }
            a = polyalpha;
        } else {
            // modulate
            r = (((tr as u32 + 1) * (vr as u32 + 1) - 1) >> 6) as u8;
            g = (((tg as u32 + 1) * (vg as u32 + 1) - 1) >> 6) as u8;
            b = (((tb as u32 + 1) * (vb as u32 + 1) - 1) >> 6) as u8;
            a = (((talpha as u32 + 1) * (polyalpha as u32 + 1) - 1) >> 5) as u8;
        }
    } else {
        r = vr;
        g = vg;
        b = vb;
        a = polyalpha;
    }

    let mut r = r;
    let mut g = g;
    let mut b = b;

    if (blendmode == 2) && (disp_cnt & (1 << 1) != 0) {
        let tooncolor = toon_table[(vr >> 1) as usize];
        // NB: the C++ recomputes vr/vg/vb from the toon table here and adds
        // them; vr was the red vertex component coming in (highlight mode set
        // vg=vb=vr above, but vr itself is unchanged from the interpolated red).
        let mut hr = ((tooncolor << 1) & 0x3E) as u8;
        if hr != 0 {
            hr += 1;
        }
        let mut hg = ((tooncolor >> 4) & 0x3E) as u8;
        if hg != 0 {
            hg += 1;
        }
        let mut hb = ((tooncolor >> 9) & 0x3E) as u8;
        if hb != 0 {
            hb += 1;
        }

        r = r.wrapping_add(hr);
        g = g.wrapping_add(hg);
        b = b.wrapping_add(hb);

        if r > 63 {
            r = 63;
        }
        if g > 63 {
            g = 63;
        }
        if b > 63 {
            b = 63;
        }
    }

    let a = if wireframe { 31u8 } else { a };

    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | ((a as u32) << 24)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texture::Vram;

    // Independent re-transcription of RenderPixel from GPU3D_Soft.cpp, kept
    // close to the C++ shape (i32 intermediates, explicit branches) so a fuzz
    // differential catches transcription mistakes in either version. Texturing
    // is delegated to the same texture_lookup (already fuzzed independently).
    #[allow(clippy::too_many_arguments)]
    fn reference(
        vram: &Vram,
        poly_attr: u32,
        tex_param: u32,
        tex_palette: u32,
        disp_cnt: u32,
        toon: &[u16; 32],
        vr_in: u8,
        vg_in: u8,
        vb_in: u8,
        s: i16,
        t: i16,
    ) -> u32 {
        let mut vr = vr_in as i32;
        let mut vg = vg_in as i32;
        let mut vb = vb_in as i32;
        let blendmode = (poly_attr >> 4) & 0x3;
        let polyalpha = ((poly_attr >> 16) & 0x1F) as i32;
        let wireframe = polyalpha == 0;
        let toon_at = |idx: i32| -> i32 { toon[idx as usize] as i32 };

        if blendmode == 2 {
            if disp_cnt & (1 << 1) != 0 {
                vg = vr;
                vb = vr;
            } else {
                let tc = toon_at(vr >> 1);
                vr = (tc << 1) & 0x3E;
                if vr != 0 { vr += 1; }
                vg = (tc >> 4) & 0x3E;
                if vg != 0 { vg += 1; }
                vb = (tc >> 9) & 0x3E;
                if vb != 0 { vb += 1; }
            }
        }

        let (mut r, mut g, mut b, a);
        if (disp_cnt & 1 != 0) && (((tex_param >> 26) & 0x7) != 0) {
            let (tcolor, talpha) = texture_lookup(vram, tex_param, tex_palette, s, t);
            let tcolor = tcolor as i32;
            let talpha = talpha as i32;
            let mut tr = (tcolor << 1) & 0x3E;
            if tr != 0 { tr += 1; }
            let mut tg = (tcolor >> 4) & 0x3E;
            if tg != 0 { tg += 1; }
            let mut tb = (tcolor >> 9) & 0x3E;
            if tb != 0 { tb += 1; }

            if blendmode & 1 != 0 {
                if talpha == 0 {
                    r = vr; g = vg; b = vb;
                } else if talpha == 31 {
                    r = tr; g = tg; b = tb;
                } else {
                    r = ((tr * talpha) + (vr * (31 - talpha))) >> 5;
                    g = ((tg * talpha) + (vg * (31 - talpha))) >> 5;
                    b = ((tb * talpha) + (vb * (31 - talpha))) >> 5;
                }
                a = polyalpha;
            } else {
                r = ((tr + 1) * (vr + 1) - 1) >> 6;
                g = ((tg + 1) * (vg + 1) - 1) >> 6;
                b = ((tb + 1) * (vb + 1) - 1) >> 6;
                a = ((talpha + 1) * (polyalpha + 1) - 1) >> 5;
            }
        } else {
            r = vr; g = vg; b = vb; a = polyalpha;
        }

        if (blendmode == 2) && (disp_cnt & (1 << 1) != 0) {
            let tc = toon_at(vr >> 1);
            let mut hr = (tc << 1) & 0x3E;
            if hr != 0 { hr += 1; }
            let mut hg = (tc >> 4) & 0x3E;
            if hg != 0 { hg += 1; }
            let mut hb = (tc >> 9) & 0x3E;
            if hb != 0 { hb += 1; }
            r = (r + hr).min(63);
            g = (g + hg).min(63);
            b = (b + hb).min(63);
        }

        let a = if wireframe { 31 } else { a };
        // mask to a byte each like the C++ u8 result assembly
        ((r & 0xFF) | ((g & 0xFF) << 8) | ((b & 0xFF) << 16) | ((a & 0xFF) << 24)) as u32
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
    fn fuzz_render_pixel_matches_reference() {
        let mut seed = 0xC0FFEE123456789u64;
        let mut tex = vec![0u8; 512 * 1024];
        let mut pal = vec![0u8; 128 * 1024];
        for x in tex.iter_mut() { *x = xorshift(&mut seed) as u8; }
        for x in pal.iter_mut() { *x = xorshift(&mut seed) as u8; }
        let vram = Vram { tex: &tex, pal: &pal };

        for _ in 0..2_000_000u64 {
            let r = xorshift(&mut seed);
            // Random poly_attr but bias blendmode/alpha across full range.
            let poly_attr = (r as u32 & 0x00FF_00FF)
                | (((r >> 20) & 0x3) as u32) << 4   // blendmode
                | (((r >> 24) & 0x1F) as u32) << 16; // polyalpha
            let tex_param = xorshift(&mut seed) as u32;
            let tex_palette = (xorshift(&mut seed) & 0x1FFF) as u32;
            let disp_cnt = (xorshift(&mut seed) & 0xF) as u32; // low render flags
            let mut toon = [0u16; 32];
            for e in toon.iter_mut() { *e = xorshift(&mut seed) as u16; }
            let vr = (xorshift(&mut seed) & 0x3F) as u8;
            let vg = (xorshift(&mut seed) & 0x3F) as u8;
            let vb = (xorshift(&mut seed) & 0x3F) as u8;
            let s = xorshift(&mut seed) as i16;
            let t = xorshift(&mut seed) as i16;

            let got = render_pixel(&vram, poly_attr, tex_param, tex_palette, disp_cnt, &toon, vr, vg, vb, s, t);
            let want = reference(&vram, poly_attr, tex_param, tex_palette, disp_cnt, &toon, vr, vg, vb, s, t);
            assert_eq!(got, want, "poly_attr={poly_attr:08x} tex_param={tex_param:08x} disp_cnt={disp_cnt:x} vr={vr} vg={vg} vb={vb} s={s} t={t}");
        }
    }
}
