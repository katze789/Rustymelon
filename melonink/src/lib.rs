//! melonink — bit-exact Rust ports of hot melonDS scanline paths.
//!
//! The only entry point so far is `melonink_composite_scanline`, a drop-in
//! replacement for the per-pixel ColorComposite loop at the end of
//! `GPU2D::SoftRenderer::DrawScanline_BGOBJ` (non-accelerated path) in
//! melonDS `src/GPU2D_Soft.cpp`. Semantics are transcribed 1:1 from that
//! file (melonDS is GPL-3.0; this crate links into the same binary).
//!
//! Behavior preservation is the contract: any deviation from the C++ code,
//! however small, is a bug here — game-visible output must be identical.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(all(not(feature = "std"), not(test)))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // No allocation, no unwinding: this library must never panic in practice
    // (all code paths are total). Abort hard if the impossible happens.
    loop {}
}

// With panic="abort" the unwinding personality is never invoked, but the
// prebuilt `core` crate still emits references to it from some objects. Supply
// a no-op so the staticlib links cleanly into the C++ core.
#[cfg(all(not(feature = "std"), not(test)))]
#[no_mangle]
extern "C" fn rust_eh_personality() {}

mod bg;
mod bg_affine;
mod composite;
mod rasterizer;
mod renderpixel;
mod simd;
mod sprite;
mod texture;

pub use composite::{composite_scalar, Regs};
pub use renderpixel::render_pixel;
pub use texture::{texture_lookup, Vram};

/// Bit-exact replacement for GPU3D SoftRenderer::TextureLookup.
/// `tex` points at VRAMFlat_Texture (>= 512 KiB), `pal` at VRAMFlat_TexPal
/// (>= 128 KiB). Writes BGR555 color and 0..=31 alpha through the out-params.
///
/// # Safety
/// `tex` must be valid for 512 KiB and `pal` for 128 KiB; out-pointers valid.
#[no_mangle]
pub unsafe extern "C" fn melonink_texture_lookup(
    tex: *const u8,
    pal: *const u8,
    texparam: u32,
    texpal: u32,
    s: i16,
    t: i16,
    out_color: *mut u16,
    out_alpha: *mut u8,
) {
    let vram = Vram {
        tex: core::slice::from_raw_parts(tex, 512 * 1024),
        pal: core::slice::from_raw_parts(pal, 128 * 1024),
    };
    let (color, alpha) = texture_lookup(&vram, texparam, texpal, s, t);
    *out_color = color;
    *out_alpha = alpha;
}

/// Bit-exact replacement for GPU3D SoftRenderer::RenderPixel. Returns the
/// shaded pixel `r | g<<8 | b<<16 | a<<24`. `toon_table` points at the 32-entry
/// RenderToonTable; `tex`/`pal` are the VRAM arrays (see texture lookup).
///
/// # Safety
/// `tex` valid for 512 KiB, `pal` for 128 KiB, `toon_table` for 32 × u16.
#[no_mangle]
pub unsafe extern "C" fn melonink_render_pixel(
    tex: *const u8,
    pal: *const u8,
    poly_attr: u32,
    tex_param: u32,
    tex_palette: u32,
    disp_cnt: u32,
    toon_table: *const u16,
    vr: u8,
    vg: u8,
    vb: u8,
    s: i16,
    t: i16,
) -> u32 {
    let vram = Vram {
        tex: core::slice::from_raw_parts(tex, 512 * 1024),
        pal: core::slice::from_raw_parts(pal, 128 * 1024),
    };
    let toon = &*(toon_table as *const [u16; 32]);
    render_pixel(
        &vram, poly_attr, tex_param, tex_palette, disp_cnt, toon, vr, vg, vb, s, t,
    )
}

/// Replaces the loop:
/// ```cpp
/// for (int i = 0; i < 256; i++)
///     BGOBJLine[i] = ColorComposite(i, BGOBJLine[i], BGOBJLine[256+i]);
/// ```
/// `bgobj_line` points at the unit's BGOBJLine buffer (>= 512 u32s),
/// `window_mask` at the unit's WindowMask buffer (>= 256 u8s).
/// Registers are passed by value; they are constant across the scanline.
///
/// # Safety
/// Caller guarantees both pointers are valid for the lengths above.
#[no_mangle]
pub unsafe extern "C" fn melonink_composite_scanline(
    bgobj_line: *mut u32,
    window_mask: *const u8,
    blend_cnt: u32,
    eva: u32,
    evb: u32,
    evy: u32,
) {
    let line = core::slice::from_raw_parts_mut(bgobj_line, 512);
    let wmask = core::slice::from_raw_parts(window_mask, 256);
    let regs = composite::Regs {
        blend_cnt,
        eva,
        evb,
        evy,
    };

    #[cfg(target_arch = "x86_64")]
    {
        if simd::avx2_available() {
            // SAFETY: feature checked above; buffers are the right size.
            simd::composite_line_avx2(line, wmask, &regs);
            return;
        }
    }

    // BlendCnt == 0 disables every effect path: the line is unchanged.
    if blend_cnt == 0 {
        return;
    }

    for i in 0..256 {
        line[i] = composite::composite_scalar(line[i], line[256 + i], wmask[i], &regs);
    }
}

#[cfg(test)]
mod tests {
    use super::composite::{composite_scalar, Regs};

    /// Independent re-transcription of ColorComposite from GPU2D_Soft.cpp /
    /// GPU2D_Soft.h, kept deliberately literal (same names, same shape) so it
    /// can be diffed against the C++ by eye.
    fn reference(i: usize, val1: u32, val2: u32, window_mask: &[u8], r: &Regs) -> u32 {
        // C unsigned arithmetic == wrapping; written with explicit u64
        // intermediates + truncation to stay independent of the impl's
        // wrapping_* formulation.
        fn wmul(a: u32, b: u32) -> u32 {
            ((a as u64 * b as u64) & 0xFFFF_FFFF) as u32
        }
        fn wadd(a: u32, b: u32) -> u32 {
            ((a as u64 + b as u64) & 0xFFFF_FFFF) as u32
        }
        fn wsub(a: u32, b: u32) -> u32 {
            ((a as i64 - b as i64) as u64 & 0xFFFF_FFFF) as u32
        }
        fn color_blend4(val1: u32, val2: u32, eva: u32, evb: u32) -> u32 {
            let mut r = wadd(wadd(wmul(val1 & 0x00003F, eva), wmul(val2 & 0x00003F, evb)), 0x000008) >> 4;
            let mut g = (wadd(wadd(wmul(val1 & 0x003F00, eva), wmul(val2 & 0x003F00, evb)), 0x000800) >> 4) & 0x007F00;
            let mut b = (wadd(wadd(wmul(val1 & 0x3F0000, eva), wmul(val2 & 0x3F0000, evb)), 0x080000) >> 4) & 0x7F0000;
            if r > 0x00003F {
                r = 0x00003F;
            }
            if g > 0x003F00 {
                g = 0x003F00;
            }
            if b > 0x3F0000 {
                b = 0x3F0000;
            }
            r | g | b | 0xFF000000
        }
        fn color_blend5(val1: u32, val2: u32) -> u32 {
            let eva = ((val1 >> 24) & 0x1F) + 1;
            let evb = 32 - eva;
            if eva == 32 {
                return val1;
            }
            let mut r = wadd(wadd(wmul(val1 & 0x00003F, eva), wmul(val2 & 0x00003F, evb)), 0x000010) >> 5;
            let mut g = (wadd(wadd(wmul(val1 & 0x003F00, eva), wmul(val2 & 0x003F00, evb)), 0x001000) >> 5) & 0x007F00;
            let mut b = (wadd(wadd(wmul(val1 & 0x3F0000, eva), wmul(val2 & 0x3F0000, evb)), 0x100000) >> 5) & 0x7F0000;
            if r > 0x00003F {
                r = 0x00003F;
            }
            if g > 0x003F00 {
                g = 0x003F00;
            }
            if b > 0x3F0000 {
                b = 0x3F0000;
            }
            r | g | b | 0xFF000000
        }
        fn brightness_up(val: u32, factor: u32, bias: u32) -> u32 {
            let rb = val & 0x3F003F;
            let g = val & 0x003F00;
            let rb = wadd(rb, (wadd(wmul(wsub(0x3F003F, rb), factor), wmul(bias, 0x010001)) >> 4) & 0x3F003F);
            let g = wadd(g, (wadd(wmul(wsub(0x003F00, g), factor), wmul(bias, 0x000100)) >> 4) & 0x003F00);
            rb | g | 0xFF000000
        }
        fn brightness_down(val: u32, factor: u32, bias: u32) -> u32 {
            let rb = val & 0x3F003F;
            let g = val & 0x003F00;
            let rb = wsub(rb, (wadd(wmul(rb, factor), wmul(bias, 0x010001)) >> 4) & 0x3F003F);
            let g = wsub(g, (wadd(wmul(g, factor), wmul(bias, 0x000100)) >> 4) & 0x003F00);
            rb | g | 0xFF000000
        }

        let mut coloreffect;
        let mut eva = 0u32;
        let mut evb = 0u32;
        let mut flag1 = val1 >> 24;
        let flag2 = val2 >> 24;
        let blend_cnt = r.blend_cnt;

        let target2 = if flag2 & 0x80 != 0 {
            0x1000
        } else if flag2 & 0x40 != 0 {
            0x0100
        } else {
            flag2 << 8
        };

        if (flag1 & 0x80 != 0) && (blend_cnt & target2 != 0) {
            coloreffect = 1;
            if flag1 & 0x40 != 0 {
                eva = flag1 & 0x1F;
                evb = wsub(16, eva);
            } else {
                eva = r.eva;
                evb = r.evb;
            }
        } else if (flag1 & 0x40 != 0) && (blend_cnt & target2 != 0) {
            coloreffect = 4;
        } else {
            if flag1 & 0x80 != 0 {
                flag1 = 0x10;
            } else if flag1 & 0x40 != 0 {
                flag1 = 0x01;
            }
            coloreffect = 0;
            if (blend_cnt & flag1 != 0) && (window_mask[i] & 0x20 != 0) {
                coloreffect = (blend_cnt >> 6) & 0x3;
                if coloreffect == 1 {
                    if blend_cnt & target2 != 0 {
                        eva = r.eva;
                        evb = r.evb;
                    } else {
                        coloreffect = 0;
                    }
                }
            }
        }

        match coloreffect {
            1 => color_blend4(val1, val2, eva, evb),
            2 => brightness_up(val1, r.evy, 0x8),
            3 => brightness_down(val1, r.evy, 0x7),
            4 => color_blend5(val1, val2),
            _ => val1,
        }
    }

    fn xorshift(state: &mut u64) -> u64 {
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *state = x;
        x
    }

    #[test]
    fn scalar_matches_reference_exhaustive_flags() {
        // Flags drive every branch; colors only flow through arithmetic.
        // Sweep all flag byte pairs x several register configs x both window
        // states, with varied color bits.
        let regcfgs = [
            Regs { blend_cnt: 0x0000, eva: 16, evb: 0, evy: 0 },
            Regs { blend_cnt: 0x3FFF, eva: 7, evb: 9, evy: 5 },
            Regs { blend_cnt: 0x0040 | 0x001F, eva: 31, evb: 31, evy: 16 },
            Regs { blend_cnt: 0x0080 | 0x1F00, eva: 0, evb: 16, evy: 31 },
            Regs { blend_cnt: 0x00C0 | 0x0234, eva: 12, evb: 4, evy: 9 },
            Regs { blend_cnt: 0x0100 | 0x0E81, eva: 1, evb: 15, evy: 2 },
        ];
        let mut rng = 0x12345678ABCDEFu64;
        for regs in &regcfgs {
            for f1 in 0..=255u32 {
                for f2 in 0..=255u32 {
                    for wm in [0u8, 0x20u8] {
                        let c1 = (xorshift(&mut rng) as u32) & 0x003F3F3F;
                        let c2 = (xorshift(&mut rng) as u32) & 0x003F3F3F;
                        let val1 = (f1 << 24) | c1;
                        let val2 = (f2 << 24) | c2;
                        let wmask = [wm; 256];
                        let got = composite_scalar(val1, val2, wm, regs);
                        let want = reference(0, val1, val2, &wmask, regs);
                        assert_eq!(
                            got, want,
                            "f1={f1:02x} f2={f2:02x} wm={wm:02x} val1={val1:08x} val2={val2:08x} regs={regs:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn simd_matches_scalar_random_lines() {
        if !crate::simd::avx2_available() {
            eprintln!("AVX2 not available; skipping");
            return;
        }
        let mut rng = 0xDEADBEEFCAFEu64;
        for iter in 0..2000 {
            let regs = Regs {
                blend_cnt: (xorshift(&mut rng) as u32) & 0x3FFF,
                eva: (xorshift(&mut rng) as u32) % 32,
                evb: (xorshift(&mut rng) as u32) % 32,
                evy: (xorshift(&mut rng) as u32) % 32,
            };
            let mut line = [0u32; 512];
            let mut wmask = [0u8; 256];
            for i in 0..512 {
                line[i] = xorshift(&mut rng) as u32;
            }
            for i in 0..256 {
                wmask[i] = (xorshift(&mut rng) as u32) as u8;
            }
            let mut expect = line;
            for i in 0..256 {
                expect[i] = composite_scalar(line[i], line[256 + i], wmask[i], &regs);
            }
            let mut got = line;
            unsafe { crate::simd::composite_line_avx2(&mut got, &wmask, &regs) };
            for i in 0..512 {
                assert_eq!(
                    got[i], expect[i],
                    "iter={iter} i={i} val1={:08x} val2={:08x} wm={:02x} regs={regs:?}",
                    line[i % 256], line[256 + (i % 256)], wmask[i % 256]
                );
            }
        }
    }
}
