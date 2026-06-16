//! Bit-exact Rust port of the per-pixel loops of
//! `GPU2D::SoftRenderer::DrawBG_Text<mosaic, drawPixel>`
//! (melonDS `src/GPU2D_Soft.cpp`).
//!
//! The entangled setup — `GetBGVRAM`, `GetBGExtPal`, tilemap/tileset address
//! maths — stays in C++; this function receives the resolved VRAM pointer +
//! mask, the base palette, the 16 possible ext-palette pointers, and the
//! per-pixel state, then runs the 256-colour and 16-colour pixel loops exactly
//! as the C++ does. `mosaic` and the `DrawPixel_Normal`/`_Accel` choice are
//! passed as runtime flags.
//!
//! `xoff` is a `u16` in the C++ and wraps at 0x10000 as it increments across
//! the scanline; that wraparound is replicated here.

#[repr(C)]
pub struct BgTextParams {
    pub bgvram: *const u8,
    pub pal: *const u16,
    pub window_mask: *const u8,
    pub bgobj_line: *mut u32,
    pub mosaic_table: *const u8,
    pub extpal_ptrs: [*const u16; 16],
    pub bgvrammask: u32,
    pub bgnum: u32,
    pub xoff: u32,
    pub yoff: u32,
    pub tilesetaddr: u32,
    pub tilemapaddr: u32,
    pub widexmask: u32,
    pub is_256: u8,
    pub extpal: u8,
    pub mosaic: u8,
    pub accel: u8,
}

#[inline]
fn expand_color(color: u16, flag: u32) -> u32 {
    let r = ((color & 0x001F) << 1) as u32;
    let g = ((color & 0x03E0) >> 4) as u32;
    let b = ((color & 0x7C00) >> 9) as u32;
    r | (g << 8) | (b << 16) | flag
}

/// # Safety
/// All pointers valid for the documented lengths (bgvram >= mask+2, pal/extpal
/// >= 256 u16, window_mask >= 256, bgobj_line >= 768, mosaic_table >= 256).
#[no_mangle]
pub unsafe extern "C" fn melonink_drawbg_text(p: *const BgTextParams) {
    let p = &*p;
    let bgvram = p.bgvram;
    let mask = p.bgvrammask as usize;
    let wmask = core::slice::from_raw_parts(p.window_mask, 256);
    let bgobj = core::slice::from_raw_parts_mut(p.bgobj_line, 768);
    let mosaic = p.mosaic != 0;
    let accel = p.accel != 0;
    let extpal = p.extpal != 0;
    let bgnum = p.bgnum;
    let yoff = p.yoff;
    let tilesetaddr = p.tilesetaddr;
    let tilemapaddr = p.tilemapaddr;
    let widexmask = p.widexmask;
    let flag = 0x01000000u32 << bgnum;
    let winbit = 1u8 << bgnum;

    let rd_u8 = |a: u32| -> u8 { *bgvram.add((a as usize) & mask) };
    let rd_u16 = |a: u32| -> u16 {
        let i = (a as usize) & mask;
        (*bgvram.add(i) as u16) | ((*bgvram.add((i + 1) & mask) as u16) << 8)
    };

    // curpal is a base pointer into 16-bit palette memory; curpal[color].
    let mut curtile: u16 = 0;
    let mut curpal: *const u16 = p.pal;
    let mut pixelsaddr: u32 = 0;

    let resolve_pal_256 = |curtile: u16| -> *const u16 {
        if extpal {
            p.extpal_ptrs[(curtile >> 12) as usize]
        } else {
            p.pal
        }
    };

    let mut xoff = p.xoff & 0xFFFF; // u16 range
    let mut lastxpos: u32 = 0;

    let draw = |bgobj: &mut [u32], i: usize, color16: u16| {
        let px = expand_color(color16, flag);
        if accel {
            bgobj[i + 512] = bgobj[i + 256];
            bgobj[i + 256] = bgobj[i];
            bgobj[i] = px;
        } else {
            bgobj[i + 256] = bgobj[i];
            bgobj[i] = px;
        }
    };

    if p.is_256 != 0 {
        // 256-color
        if (xoff & 0x7) != 0 || mosaic {
            curtile = rd_u16(tilemapaddr + ((xoff & 0xF8) >> 2) + ((xoff & widexmask) << 3));
            curpal = resolve_pal_256(curtile);
            pixelsaddr = tilesetaddr
                + ((curtile as u32 & 0x03FF) << 6)
                + ((if curtile & 0x0800 != 0 { 7 - (yoff & 0x7) } else { yoff & 0x7 }) << 3);
        }
        if mosaic {
            lastxpos = xoff;
        }

        for i in 0..256 {
            let xpos = if mosaic {
                xoff.wrapping_sub(*p.mosaic_table.add(i) as u32)
            } else {
                xoff
            };

            if (!mosaic && (xpos & 0x7) == 0) || (mosaic && (xpos >> 3) != (lastxpos >> 3)) {
                curtile = rd_u16(tilemapaddr + ((xpos & 0xF8) >> 2) + ((xpos & widexmask) << 3));
                curpal = resolve_pal_256(curtile);
                pixelsaddr = tilesetaddr
                    + ((curtile as u32 & 0x03FF) << 6)
                    + ((if curtile & 0x0800 != 0 { 7 - (yoff & 0x7) } else { yoff & 0x7 }) << 3);
                if mosaic {
                    lastxpos = xpos;
                }
            }

            if wmask[i] & winbit != 0 {
                let tilexoff = if curtile & 0x0400 != 0 { 7 - (xpos & 0x7) } else { xpos & 0x7 };
                let color = rd_u8(pixelsaddr + tilexoff);
                if color != 0 {
                    draw(bgobj, i, *curpal.add(color as usize));
                }
            }

            xoff = (xoff + 1) & 0xFFFF;
        }
    } else {
        // 16-color
        if (xoff & 0x7) != 0 || mosaic {
            curtile = rd_u16(tilemapaddr + ((xoff & 0xF8) >> 2) + ((xoff & widexmask) << 3));
            curpal = p.pal.add(((curtile as u32 & 0xF000) >> 8) as usize);
            pixelsaddr = tilesetaddr
                + ((curtile as u32 & 0x03FF) << 5)
                + ((if curtile & 0x0800 != 0 { 7 - (yoff & 0x7) } else { yoff & 0x7 }) << 2);
        }
        if mosaic {
            lastxpos = xoff;
        }

        for i in 0..256 {
            let xpos = if mosaic {
                xoff.wrapping_sub(*p.mosaic_table.add(i) as u32)
            } else {
                xoff
            };

            if (!mosaic && (xpos & 0x7) == 0) || (mosaic && (xpos >> 3) != (lastxpos >> 3)) {
                curtile = rd_u16(tilemapaddr + ((xpos & 0xF8) >> 2) + ((xpos & widexmask) << 3));
                curpal = p.pal.add(((curtile as u32 & 0xF000) >> 8) as usize);
                pixelsaddr = tilesetaddr
                    + ((curtile as u32 & 0x03FF) << 5)
                    + ((if curtile & 0x0800 != 0 { 7 - (yoff & 0x7) } else { yoff & 0x7 }) << 2);
                if mosaic {
                    lastxpos = xpos;
                }
            }

            if wmask[i] & winbit != 0 {
                let tilexoff = if curtile & 0x0400 != 0 { 7 - (xpos & 0x7) } else { xpos & 0x7 };
                let color = if tilexoff & 0x1 != 0 {
                    rd_u8(pixelsaddr + (tilexoff >> 1)) >> 4
                } else {
                    rd_u8(pixelsaddr + (tilexoff >> 1)) & 0x0F
                };
                if color != 0 {
                    draw(bgobj, i, *curpal.add(color as usize));
                }
            }

            xoff = (xoff + 1) & 0xFFFF;
        }
    }
}
