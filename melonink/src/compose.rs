//! Bit-exact Rust ports of the small 2D-composition helpers in
//! `GPU2D::SoftRenderer` (melonDS `src/GPU2D_Soft.cpp`):
//! `InterleaveSprites`, `DrawBG_3D`, and `ApplySpriteMosaicX`.

#[inline]
fn expand(color: u16, flag: u32) -> u32 {
    let r = ((color & 0x001F) << 1) as u32;
    let g = ((color & 0x03E0) >> 4) as u32;
    let b = ((color & 0x7C00) >> 9) as u32;
    r | (g << 8) | (b << 16) | flag
}

#[repr(C)]
pub struct InterleaveParams {
    pub obj_line: *const u32,
    pub pal: *const u16,
    pub extpal: *const u16,
    pub window_mask: *const u8,
    pub bgobj_line: *mut u32,
    pub prio: u32,
    pub use_extpal: u8,
    pub accel: u8,
    pub _pad0: u8,
    pub _pad1: u8,
}

/// Port of `InterleaveSprites<drawPixel>(prio)`: resolves OBJ-line palette
/// indices to colours and composites them into BGOBJLine.
///
/// # Safety
/// obj_line/window_mask >= 256; pal/extpal >= 4096 u16 (extpal indexed up to
/// 0xFFF); bgobj_line >= 768.
#[no_mangle]
pub unsafe extern "C" fn melonink_interleave_sprites(p: *const InterleaveParams) {
    let p = &*p;
    let obj = core::slice::from_raw_parts(p.obj_line, 256);
    let wmask = core::slice::from_raw_parts(p.window_mask, 256);
    let bgobj = core::slice::from_raw_parts_mut(p.bgobj_line, 768);
    let accel = p.accel != 0;
    let use_extpal = p.use_extpal != 0;
    let prio = p.prio;

    for i in 0..256 {
        let pixel = obj[i];
        if pixel & 0x70000 != prio {
            continue;
        }
        if wmask[i] & 0x10 == 0 {
            continue;
        }
        let color: u16 = if pixel & 0x8000 != 0 {
            (pixel & 0x7FFF) as u16
        } else if use_extpal {
            if pixel & 0x1000 != 0 {
                *p.pal.add((pixel & 0xFF) as usize)
            } else {
                *p.extpal.add((pixel & 0xFFF) as usize)
            }
        } else {
            *p.pal.add((pixel & 0xFF) as usize)
        };
        let px = expand(color, pixel & 0xFF000000);
        if accel {
            bgobj[i + 512] = bgobj[i + 256];
            bgobj[i + 256] = bgobj[i];
            bgobj[i] = px;
        } else {
            bgobj[i + 256] = bgobj[i];
            bgobj[i] = px;
        }
    }
}

/// Port of `DrawBG_3D()`. `accelerated` selects the placeholder vs the actual
/// `_3DLine` composite (the software renderer uses the latter).
///
/// # Safety
/// bgobj_line >= 768; threed_line/window_mask >= 256.
#[no_mangle]
pub unsafe extern "C" fn melonink_drawbg_3d(
    bgobj_line: *mut u32,
    threed_line: *const u32,
    window_mask: *const u8,
    accelerated: u8,
) {
    let bgobj = core::slice::from_raw_parts_mut(bgobj_line, 768);
    let threed = core::slice::from_raw_parts(threed_line, 256);
    let wmask = core::slice::from_raw_parts(window_mask, 256);

    if accelerated != 0 {
        for i in 0..256 {
            if wmask[i] & 0x01 == 0 {
                continue;
            }
            bgobj[i + 512] = bgobj[i + 256];
            bgobj[i + 256] = bgobj[i];
            bgobj[i] = 0x40000000;
        }
    } else {
        for i in 0..256 {
            let c = threed[i];
            if (c >> 24) == 0 {
                continue;
            }
            if wmask[i] & 0x01 == 0 {
                continue;
            }
            bgobj[i + 256] = bgobj[i];
            bgobj[i] = c | 0x40000000;
        }
    }
}

#[repr(C)]
pub struct CaptureParams {
    pub dst: *mut u16,
    pub src_a: *const u32,
    pub src_b: *const u16,
    pub dstaddr: u32,
    pub src_baddr: u32,
    pub width: u32,
    pub mode: u32,
    pub eva: u32,
    pub evb: u32,
    pub has_srcb: u8,
    pub _pad0: u8,
    pub _pad1: u8,
    pub _pad2: u8,
}

/// Port of the per-pixel capture loops of `DoCapture` (the `switch` on capture
/// mode). VRAM/FIFO resolution, dirty-marking, and the OpenGL-only composite
/// block stay in C++; this writes the captured pixels into VRAM.
///
/// # Safety
/// dst/src_b valid for 0x10000 u16 (addresses wrap & 0xFFFF); src_a >= width.
#[no_mangle]
pub unsafe extern "C" fn melonink_do_capture(p: *const CaptureParams) {
    let p = &*p;
    let dst = core::slice::from_raw_parts_mut(p.dst, 0x10000);
    let src_a = core::slice::from_raw_parts(p.src_a, p.width.max(1) as usize);
    let has_srcb = p.has_srcb != 0;
    let mut dstaddr = p.dstaddr;
    let mut src_baddr = p.src_baddr;
    let eva = p.eva;
    let evb = p.evb;

    match p.mode {
        0 => {
            // source A
            for i in 0..p.width as usize {
                let val = src_a[i];
                let r = (val >> 1) & 0x1F;
                let g = (val >> 9) & 0x1F;
                let b = (val >> 17) & 0x1F;
                let a = if (val >> 24) != 0 { 0x8000 } else { 0 };
                dst[dstaddr as usize] = (r | (g << 5) | (b << 10) | a) as u16;
                dstaddr = (dstaddr + 1) & 0xFFFF;
            }
        }
        1 => {
            // source B
            if has_srcb {
                let src_b = core::slice::from_raw_parts(p.src_b, 0x10000);
                for _ in 0..p.width {
                    dst[dstaddr as usize] = src_b[src_baddr as usize];
                    src_baddr = (src_baddr + 1) & 0xFFFF;
                    dstaddr = (dstaddr + 1) & 0xFFFF;
                }
            } else {
                for _ in 0..p.width {
                    dst[dstaddr as usize] = 0;
                    dstaddr = (dstaddr + 1) & 0xFFFF;
                }
            }
        }
        _ => {
            // sources A+B (modes 2 and 3)
            if has_srcb {
                let src_b = core::slice::from_raw_parts(p.src_b, 0x10000);
                for i in 0..p.width as usize {
                    let val = src_a[i];
                    let r_a = (val >> 1) & 0x1F;
                    let g_a = (val >> 9) & 0x1F;
                    let b_a = (val >> 17) & 0x1F;
                    let a_a = if (val >> 24) != 0 { 1 } else { 0 };
                    let valb = src_b[src_baddr as usize] as u32;
                    let r_b = valb & 0x1F;
                    let g_b = (valb >> 5) & 0x1F;
                    let b_b = (valb >> 10) & 0x1F;
                    let a_b = valb >> 15;
                    let mut r_d = ((r_a * a_a * eva) + (r_b * a_b * evb) + 8) >> 4;
                    let mut g_d = ((g_a * a_a * eva) + (g_b * a_b * evb) + 8) >> 4;
                    let mut b_d = ((b_a * a_a * eva) + (b_b * a_b * evb) + 8) >> 4;
                    let a_d = (if eva > 0 { a_a } else { 0 }) | (if evb > 0 { a_b } else { 0 });
                    if r_d > 0x1F {
                        r_d = 0x1F;
                    }
                    if g_d > 0x1F {
                        g_d = 0x1F;
                    }
                    if b_d > 0x1F {
                        b_d = 0x1F;
                    }
                    dst[dstaddr as usize] = (r_d | (g_d << 5) | (b_d << 10) | (a_d << 15)) as u16;
                    src_baddr = (src_baddr + 1) & 0xFFFF;
                    dstaddr = (dstaddr + 1) & 0xFFFF;
                }
            } else {
                for i in 0..p.width as usize {
                    let val = src_a[i];
                    let r_a = (val >> 1) & 0x1F;
                    let g_a = (val >> 9) & 0x1F;
                    let b_a = (val >> 17) & 0x1F;
                    let a_a = if (val >> 24) != 0 { 1 } else { 0 };
                    let r_d = ((r_a * a_a * eva) + 8) >> 4;
                    let g_d = ((g_a * a_a * eva) + 8) >> 4;
                    let b_d = ((b_a * a_a * eva) + 8) >> 4;
                    let a_d = if eva > 0 { a_a } else { 0 };
                    dst[dstaddr as usize] = (r_d | (g_d << 5) | (b_d << 10) | (a_d << 15)) as u16;
                    dstaddr = (dstaddr + 1) & 0xFFFF;
                }
            }
        }
    }
}

/// Port of `ApplySpriteMosaicX()`. The caller (C++) handles the early-out for
/// mosaic size 0 and provides the resolved mosaic table.
///
/// # Safety
/// obj_line/mosaic_table >= 256.
#[no_mangle]
pub unsafe extern "C" fn melonink_apply_sprite_mosaic_x(
    obj_line: *mut u32,
    mosaic_table: *const u8,
) {
    let obj = core::slice::from_raw_parts_mut(obj_line, 256);
    let table = core::slice::from_raw_parts(mosaic_table, 256);
    let mut lastcolor = obj[0];
    for i in 1..256 {
        let currentcolor = obj[i];
        if (lastcolor & currentcolor & 0x100000) == 0 || table[i] == 0 {
            lastcolor = currentcolor;
        } else {
            obj[i] = lastcolor;
        }
    }
}
