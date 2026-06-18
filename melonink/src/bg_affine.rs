//! Bit-exact Rust port of the affine/extended/large 2D background pixel loops:
//! `GPU2D::SoftRenderer::DrawBG_Affine`, `DrawBG_Extended`, and `DrawBG_Large`
//! (melonDS `src/GPU2D_Soft.cpp`).
//!
//! All three share the same affine-sampling shape (a per-pixel rotX/rotY walk,
//! optional X mosaic, window masking, overflow masking) and differ only in how
//! the VRAM address and palette are computed. They are dispatched here by
//! `mode`. The entangled setup (`GetBGVRAM`/`GetBGExtPal`, the register maths)
//! and the once-per-scanline `BGxRefInternal += rotB/rotD` update stay in C++.

/// Drawing mode (mirrors which C++ branch produced the params).
pub const MODE_AFFINE_TILED: u32 = 0; // DrawBG_Affine
pub const MODE_EXT_BITMAP_DIRECT: u32 = 1; // DrawBG_Extended, bitmap, direct colour
pub const MODE_EXT_BITMAP_256: u32 = 2; // DrawBG_Extended, bitmap, 256-colour
pub const MODE_EXT_MIXED: u32 = 3; // DrawBG_Extended, affine/text (+extpal)
pub const MODE_LARGE_256: u32 = 4; // DrawBG_Large

#[repr(C)]
pub struct BgAffineParams {
    pub bgvram: *const u8,
    pub pal: *const u16,
    pub window_mask: *const u8,
    pub bgobj_line: *mut u32,
    pub mosaic_table: *const u8,
    pub extpal_ptrs: [*const u16; 16],
    pub bgvrammask: u32,
    pub bgnum: u32,
    pub mode: u32,
    pub tileset_addr: u32,
    pub tilemap_addr: u32,
    pub mask_x: u32, // coordmask (tiled) or xmask (bitmap)
    pub mask_y: u32, // ymask (bitmap modes)
    pub yshift: u32,
    pub ovmask_x: u32, // overflowmask (tiled) or ofxmask (bitmap)
    pub ovmask_y: u32, // ofymask (bitmap)
    pub rot_x: i32,
    pub rot_y: i32,
    pub rot_a: i32,
    pub rot_c: i32,
    pub mosaic: u8,
    pub accel: u8,
    pub extpal: u8,
    pub _pad: u8,
}

#[inline]
fn expand(color: u16, flag: u32) -> u32 {
    let r = ((color & 0x001F) << 1) as u32;
    let g = ((color & 0x03E0) >> 4) as u32;
    let b = ((color & 0x7C00) >> 9) as u32;
    r | (g << 8) | (b << 16) | flag
}

/// # Safety
/// All pointers valid for the documented lengths (bgvram >= mask+2, pal/extpal
/// >= 256 u16, window_mask >= 256, bgobj_line >= 768, mosaic_table >= 256).
#[no_mangle]
pub unsafe extern "C" fn melonink_drawbg_affine(p: *const BgAffineParams) {
    let p = &*p;
    let bgvram = p.bgvram;
    let mask = p.bgvrammask;
    let wmask = core::slice::from_raw_parts(p.window_mask, 256);
    let bgobj = core::slice::from_raw_parts_mut(p.bgobj_line, 768);
    let mosaic = p.mosaic != 0;
    let accel = p.accel != 0;
    let extpal = p.extpal != 0;
    let bgnum = p.bgnum;
    let flag = 0x01000000u32 << bgnum;
    let winbit = 1u8 << bgnum;
    let yshift = p.yshift;

    let rd8 = |a: u32| -> u8 { *bgvram.add((a & mask) as usize) };
    let rd16 = |a: u32| -> u16 {
        let i = (a & mask) as usize;
        (*bgvram.add(i) as u16) | ((*bgvram.add((i + 1) & mask as usize) as u16) << 8)
    };

    let mut rot_x = p.rot_x;
    let mut rot_y = p.rot_y;

    let draw = |bgobj: &mut [u32], i: usize, color16: u16| {
        let px = expand(color16, flag);
        if accel {
            bgobj[i + 512] = bgobj[i + 256];
            bgobj[i + 256] = bgobj[i];
            bgobj[i] = px;
        } else {
            bgobj[i + 256] = bgobj[i];
            bgobj[i] = px;
        }
    };

    for i in 0..256 {
        if wmask[i] & winbit != 0 {
            let (final_x, final_y) = if mosaic {
                let im = *p.mosaic_table.add(i) as i32;
                (
                    rot_x.wrapping_sub(im.wrapping_mul(p.rot_a)),
                    rot_y.wrapping_sub(im.wrapping_mul(p.rot_c)),
                )
            } else {
                (rot_x, rot_y)
            };
            let fx = final_x as u32;
            let fy = final_y as u32;

            match p.mode {
                MODE_AFFINE_TILED => {
                    if (fx | fy) & p.ovmask_x == 0 {
                        let curtile = rd8(
                            p.tilemap_addr
                                .wrapping_add((((fy & p.mask_x) >> 11) << yshift)
                                    + ((fx & p.mask_x) >> 11)),
                        ) as u32;
                        let tilexoff = ((final_x >> 8) & 0x7) as u32;
                        let tileyoff = ((final_y >> 8) & 0x7) as u32;
                        let color = rd8(
                            p.tileset_addr
                                .wrapping_add((curtile << 6) + (tileyoff << 3) + tilexoff),
                        );
                        if color != 0 {
                            draw(bgobj, i, *p.pal.add(color as usize));
                        }
                    }
                }
                MODE_EXT_BITMAP_DIRECT => {
                    if fx & p.ovmask_x == 0 && fy & p.ovmask_y == 0 {
                        let addr = p.tilemap_addr.wrapping_add(
                            ((((fy & p.mask_y) >> 8) << yshift) + ((fx & p.mask_x) >> 8)) << 1,
                        );
                        let color = rd16(addr);
                        if color & 0x8000 != 0 {
                            draw(bgobj, i, color);
                        }
                    }
                }
                MODE_EXT_BITMAP_256 => {
                    if fx & p.ovmask_x == 0 && fy & p.ovmask_y == 0 {
                        let addr = p.tilemap_addr.wrapping_add(
                            (((fy & p.mask_y) >> 8) << yshift) + ((fx & p.mask_x) >> 8),
                        );
                        let color = rd8(addr);
                        if color != 0 {
                            draw(bgobj, i, *p.pal.add(color as usize));
                        }
                    }
                }
                MODE_EXT_MIXED => {
                    if (fx | fy) & p.ovmask_x == 0 {
                        let curtile = rd16(
                            p.tilemap_addr.wrapping_add(
                                ((((fy & p.mask_x) >> 11) << yshift) + ((fx & p.mask_x) >> 11)) << 1,
                            ),
                        );
                        let curpal: *const u16 = if extpal {
                            p.extpal_ptrs[(curtile >> 12) as usize]
                        } else {
                            p.pal
                        };
                        let mut tilexoff = ((final_x >> 8) & 0x7) as u32;
                        let mut tileyoff = ((final_y >> 8) & 0x7) as u32;
                        if curtile & 0x0400 != 0 {
                            tilexoff = 7 - tilexoff;
                        }
                        if curtile & 0x0800 != 0 {
                            tileyoff = 7 - tileyoff;
                        }
                        let color = rd8(
                            p.tileset_addr.wrapping_add(
                                ((curtile as u32 & 0x03FF) << 6) + (tileyoff << 3) + tilexoff,
                            ),
                        );
                        if color != 0 {
                            draw(bgobj, i, *curpal.add(color as usize));
                        }
                    }
                }
                MODE_LARGE_256 => {
                    if fx & p.ovmask_x == 0 && fy & p.ovmask_y == 0 {
                        let addr = (((fy & p.mask_y) >> 8) << yshift) + ((fx & p.mask_x) >> 8);
                        let color = rd8(addr);
                        if color != 0 {
                            draw(bgobj, i, *p.pal.add(color as usize));
                        }
                    }
                }
                _ => {}
            }
        }

        rot_x = rot_x.wrapping_add(p.rot_a);
        rot_y = rot_y.wrapping_add(p.rot_c);
    }
}
