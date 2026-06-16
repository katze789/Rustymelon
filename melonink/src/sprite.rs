//! Bit-exact Rust port of `GPU2D::SoftRenderer::DrawSprite_Normal<window>` and
//! `DrawSprite_Rotscale<window>` (melonDS `src/GPU2D_Soft.cpp`).
//!
//! These fill `OBJLine`/`OBJWindow` with raw palette indices + attribute bits
//! for one sprite; the palette lookup happens later in `InterleaveSprites`
//! (kept in C++). So the only state needed here is the resolved OBJ VRAM
//! pointer + mask, the sprite's OAM attributes, the affine params (rotscale),
//! `DispCnt`, and the geometry the C++ dispatcher already computed.
//!
//! Pointer arithmetic on `pixelsaddr` is `u32` and wraps exactly like the C++
//! (it is always masked with `objvrammask` before use).

#[repr(C)]
pub struct SpriteParams {
    pub objvram: *const u8,
    pub obj_line: *mut u32,
    pub obj_window: *mut u8,
    pub objvrammask: u32,
    pub disp_cnt: u32,
    pub attrib0: u32,
    pub attrib1: u32,
    pub attrib2: u32,
    pub rot_a: i32,
    pub rot_b: i32,
    pub rot_c: i32,
    pub rot_d: i32,
    pub width: i32,
    pub height: i32,
    pub boundwidth: i32,
    pub boundheight: i32,
    pub xpos: i32,
    pub ypos: i32,
    pub window: u8,
    pub rotscale: u8,
    pub _pad0: u8,
    pub _pad1: u8,
}

struct Obj<'a> {
    vram: *const u8,
    mask: u32,
    line: &'a mut [u32],
    win: &'a mut [u8],
    window: bool,
}

impl Obj<'_> {
    #[inline]
    unsafe fn rd8(&self, a: u32) -> u8 {
        *self.vram.add((a & self.mask) as usize)
    }
    #[inline]
    unsafe fn rd16(&self, a: u32) -> u16 {
        let i = (a & self.mask) as usize;
        (*self.vram.add(i) as u16) | ((*self.vram.add((i + 1) & self.mask as usize) as u16) << 8)
    }
    /// The shared per-pixel store used by every sprite mode.
    #[inline]
    fn store(&mut self, xpos: usize, color: u16, opaque: bool, pixelattr: u32) {
        if opaque {
            if self.window {
                self.win[xpos] = 1;
            } else {
                self.line[xpos] = color as u32 | pixelattr;
            }
        } else if !self.window && self.line[xpos] == 0 {
            self.line[xpos] = pixelattr & 0x180000;
        }
    }
}

/// # Safety
/// Pointers valid: objvram for mask+2 bytes, obj_line/obj_window for 256 each.
#[no_mangle]
pub unsafe extern "C" fn melonink_draw_sprite(p: *const SpriteParams) {
    let p = &*p;
    let mut o = Obj {
        vram: p.objvram,
        mask: p.objvrammask,
        line: core::slice::from_raw_parts_mut(p.obj_line, 256),
        win: core::slice::from_raw_parts_mut(p.obj_window, 256),
        window: p.window != 0,
    };
    if p.rotscale != 0 {
        draw_rotscale(&mut o, p);
    } else {
        draw_normal(&mut o, p);
    }
}

unsafe fn draw_normal(o: &mut Obj, p: &SpriteParams) {
    let window = o.window;
    let a0 = p.attrib0;
    let a1 = p.attrib1;
    let a2 = p.attrib2;
    let dispcnt = p.disp_cnt;
    let width = p.width;
    let height = p.height;
    let mut ypos = p.ypos;
    let mut xpos = p.xpos;

    let mut pixelattr = ((a2 & 0x0C00) << 6) | 0xC0000;
    let tilenum = a2 & 0x03FF;
    let spritemode = if window { 0 } else { (a0 >> 10) & 0x3 };
    let wmask = (width - 8) as u32; // ((width-1) & ~0x7)

    if (a0 & 0x1000) != 0 && !window {
        pixelattr |= 0x100000;
    }

    // yflip
    if a1 & 0x2000 != 0 {
        ypos = height - 1 - ypos;
    }

    let mut xoff: u32;
    let mut xend: u32 = width as u32;
    if xpos >= 0 {
        xoff = 0;
        if (xpos as u32 + xend) > 256 {
            xend = 256 - xpos as u32;
        }
    } else {
        xoff = (-xpos) as u32;
        xpos = 0;
    }

    if spritemode == 3 {
        // bitmap sprite
        let mut alpha = a2 >> 12;
        if alpha == 0 {
            return;
        }
        alpha += 1;
        pixelattr |= 0xC0000000 | (alpha << 24);

        let mut pixelsaddr = tilenum;
        if dispcnt & 0x40 != 0 {
            if dispcnt & 0x20 != 0 {
                return; // reserved
            } else {
                pixelsaddr <<= 7 + ((dispcnt >> 22) & 0x1);
                pixelsaddr = pixelsaddr.wrapping_add((ypos as u32) * (width as u32) * 2);
            }
        } else if dispcnt & 0x20 != 0 {
            pixelsaddr = ((tilenum & 0x01F) << 4) + ((tilenum & 0x3E0) << 7);
            pixelsaddr = pixelsaddr.wrapping_add((ypos as u32) * 256 * 2);
        } else {
            pixelsaddr = ((tilenum & 0x00F) << 4) + ((tilenum & 0x3F0) << 7);
            pixelsaddr = pixelsaddr.wrapping_add((ypos as u32) * 128 * 2);
        }

        let pixelstride: i32;
        if a1 & 0x1000 != 0 {
            pixelsaddr = pixelsaddr.wrapping_add(((width - 1) as u32) << 1);
            pixelsaddr = pixelsaddr.wrapping_sub(xoff << 1);
            pixelstride = -2;
        } else {
            pixelsaddr = pixelsaddr.wrapping_add(xoff << 1);
            pixelstride = 2;
        }

        while xoff < xend {
            let color = o.rd16(pixelsaddr);
            pixelsaddr = pixelsaddr.wrapping_add(pixelstride as u32);
            o.store(xpos as usize, color, color & 0x8000 != 0, pixelattr);
            xoff += 1;
            xpos += 1;
        }
    } else {
        let mut pixelsaddr = tilenum;
        if dispcnt & 0x10 != 0 {
            pixelsaddr <<= (dispcnt >> 20) & 0x3;
            pixelsaddr = pixelsaddr
                .wrapping_add((((ypos >> 3) * (width >> 3)) << (if a0 & 0x2000 != 0 { 1 } else { 0 })) as u32);
        } else {
            pixelsaddr = pixelsaddr.wrapping_add(((ypos >> 3) * 0x20) as u32);
        }

        if spritemode == 1 {
            pixelattr |= 0x80000000;
        } else {
            pixelattr |= 0x10000000;
        }

        if a0 & 0x2000 != 0 {
            // 256-color
            pixelsaddr <<= 5;
            pixelsaddr = pixelsaddr.wrapping_add(((ypos & 0x7) << 3) as u32);
            let pixelstride: i32;

            if !window {
                if dispcnt & 0x80000000 == 0 {
                    pixelattr |= 0x1000;
                } else {
                    pixelattr |= (a2 & 0xF000) >> 4;
                }
            }

            if a1 & 0x1000 != 0 {
                pixelsaddr = pixelsaddr.wrapping_add(((width - 1) as u32 & wmask) << 3);
                pixelsaddr = pixelsaddr.wrapping_add((width - 1) as u32 & 0x7);
                pixelsaddr = pixelsaddr.wrapping_sub((xoff & wmask) << 3);
                pixelsaddr = pixelsaddr.wrapping_sub(xoff & 0x7);
                pixelstride = -1;
            } else {
                pixelsaddr = pixelsaddr.wrapping_add((xoff & wmask) << 3);
                pixelsaddr = pixelsaddr.wrapping_add(xoff & 0x7);
                pixelstride = 1;
            }

            while xoff < xend {
                let color = o.rd8(pixelsaddr) as u16;
                pixelsaddr = pixelsaddr.wrapping_add(pixelstride as u32);
                o.store(xpos as usize, color, color != 0, pixelattr);
                xoff += 1;
                xpos += 1;
                if xoff & 0x7 == 0 {
                    pixelsaddr = pixelsaddr.wrapping_add((56 * pixelstride) as u32);
                }
            }
        } else {
            // 16-color. (The C++ declares a `pixelstride` here but never uses
            // it — the loop advances pixelsaddr directly — so it is omitted.)
            pixelsaddr <<= 5;
            pixelsaddr = pixelsaddr.wrapping_add(((ypos & 0x7) << 2) as u32);

            if !window {
                pixelattr |= 0x1000;
                pixelattr |= (a2 & 0xF000) >> 8;
            }

            let xflip = a1 & 0x1000 != 0;
            if xflip {
                pixelsaddr = pixelsaddr.wrapping_add(((width - 1) as u32 & wmask) << 2);
                pixelsaddr = pixelsaddr.wrapping_add(((width - 1) as u32 & 0x7) >> 1);
                pixelsaddr = pixelsaddr.wrapping_sub((xoff & wmask) << 2);
                pixelsaddr = pixelsaddr.wrapping_sub((xoff & 0x7) >> 1);
            } else {
                pixelsaddr = pixelsaddr.wrapping_add((xoff & wmask) << 2);
                pixelsaddr = pixelsaddr.wrapping_add((xoff & 0x7) >> 1);
            }

            while xoff < xend {
                let color;
                if xflip {
                    if xoff & 0x1 != 0 {
                        color = (o.rd8(pixelsaddr) & 0x0F) as u16;
                        pixelsaddr = pixelsaddr.wrapping_sub(1);
                    } else {
                        color = (o.rd8(pixelsaddr) >> 4) as u16;
                    }
                } else if xoff & 0x1 != 0 {
                    color = (o.rd8(pixelsaddr) >> 4) as u16;
                    pixelsaddr = pixelsaddr.wrapping_add(1);
                } else {
                    color = (o.rd8(pixelsaddr) & 0x0F) as u16;
                }

                o.store(xpos as usize, color, color != 0, pixelattr);
                xoff += 1;
                xpos += 1;
                if xoff & 0x7 == 0 {
                    pixelsaddr = pixelsaddr.wrapping_add((if xflip { -28i32 } else { 28 }) as u32);
                }
            }
        }
    }
}

unsafe fn draw_rotscale(o: &mut Obj, p: &SpriteParams) {
    let window = o.window;
    let a0 = p.attrib0;
    let a2 = p.attrib2;
    let dispcnt = p.disp_cnt;
    let mut width = p.width;
    let mut height = p.height;
    let mut boundwidth = p.boundwidth;
    let boundheight = p.boundheight;
    let ypos = p.ypos;
    let mut xpos = p.xpos;

    let mut pixelattr = ((a2 & 0x0C00) << 6) | 0xC0000;
    let tilenum = a2 & 0x03FF;
    let spritemode = if window { 0 } else { (a0 >> 10) & 0x3 };
    let mut ytilefactor: u32;

    let center_x = boundwidth >> 1;
    let center_y = boundheight >> 1;

    if (a0 & 0x1000) != 0 && !window {
        pixelattr |= 0x100000;
    }

    let mut xoff: u32;
    if xpos >= 0 {
        xoff = 0;
        if (xpos + boundwidth) > 256 {
            boundwidth = 256 - xpos;
        }
    } else {
        xoff = (-xpos) as u32;
        xpos = 0;
    }

    let rot_a = p.rot_a;
    let rot_c = p.rot_c;
    let mut rot_x = ((xoff as i32 - center_x) * p.rot_a)
        + ((ypos - center_y) * p.rot_b)
        + (width << 7);
    let mut rot_y = ((xoff as i32 - center_x) * p.rot_c)
        + ((ypos - center_y) * p.rot_d)
        + (height << 7);

    width <<= 8;
    height <<= 8;

    if spritemode == 3 {
        let mut alpha = a2 >> 12;
        if alpha == 0 {
            return;
        }
        alpha += 1;
        pixelattr |= 0xC0000000 | (alpha << 24);

        let pixelsaddr: u32;
        if dispcnt & 0x40 != 0 {
            if dispcnt & 0x20 != 0 {
                return; // reserved
            } else {
                pixelsaddr = tilenum << (7 + ((dispcnt >> 22) & 0x1));
                ytilefactor = ((width >> 8) * 2) as u32;
            }
        } else if dispcnt & 0x20 != 0 {
            pixelsaddr = ((tilenum & 0x01F) << 4) + ((tilenum & 0x3E0) << 7);
            ytilefactor = 256 * 2;
        } else {
            pixelsaddr = ((tilenum & 0x00F) << 4) + ((tilenum & 0x3F0) << 7);
            ytilefactor = 128 * 2;
        }

        while xoff < boundwidth as u32 {
            if (rot_x as u32) < width as u32 && (rot_y as u32) < height as u32 {
                let addr = pixelsaddr
                    .wrapping_add(((rot_y >> 8) as u32).wrapping_mul(ytilefactor))
                    .wrapping_add(((rot_x >> 8) as u32) << 1);
                let color = o.rd16(addr);
                o.store(xpos as usize, color, color & 0x8000 != 0, pixelattr);
            }
            rot_x += rot_a;
            rot_y += rot_c;
            xoff += 1;
            xpos += 1;
        }
    } else {
        let mut pixelsaddr = tilenum;
        if dispcnt & 0x10 != 0 {
            pixelsaddr <<= (dispcnt >> 20) & 0x3;
            ytilefactor = ((width >> 11) << (if a0 & 0x2000 != 0 { 1 } else { 0 })) as u32;
        } else {
            ytilefactor = 0x20;
        }

        if spritemode == 1 {
            pixelattr |= 0x80000000;
        } else {
            pixelattr |= 0x10000000;
        }

        ytilefactor <<= 5;
        pixelsaddr <<= 5;

        if a0 & 0x2000 != 0 {
            // 256-color
            if !window {
                if dispcnt & 0x80000000 == 0 {
                    pixelattr |= 0x1000;
                } else {
                    pixelattr |= (a2 & 0xF000) >> 4;
                }
            }
            while xoff < boundwidth as u32 {
                if (rot_x as u32) < width as u32 && (rot_y as u32) < height as u32 {
                    let addr = pixelsaddr
                        .wrapping_add(((rot_y >> 11) as u32).wrapping_mul(ytilefactor))
                        .wrapping_add(((rot_y & 0x700) >> 5) as u32)
                        .wrapping_add(((rot_x >> 11) as u32).wrapping_mul(64))
                        .wrapping_add(((rot_x & 0x700) >> 8) as u32);
                    let color = o.rd8(addr) as u16;
                    o.store(xpos as usize, color, color != 0, pixelattr);
                }
                rot_x += rot_a;
                rot_y += rot_c;
                xoff += 1;
                xpos += 1;
            }
        } else {
            // 16-color
            if !window {
                pixelattr |= 0x1000;
                pixelattr |= (a2 & 0xF000) >> 8;
            }
            while xoff < boundwidth as u32 {
                if (rot_x as u32) < width as u32 && (rot_y as u32) < height as u32 {
                    let addr = pixelsaddr
                        .wrapping_add(((rot_y >> 11) as u32).wrapping_mul(ytilefactor))
                        .wrapping_add(((rot_y & 0x700) >> 6) as u32)
                        .wrapping_add(((rot_x >> 11) as u32).wrapping_mul(32))
                        .wrapping_add(((rot_x & 0x700) >> 9) as u32);
                    let mut color = o.rd8(addr) as u16;
                    if rot_x & 0x100 != 0 {
                        color >>= 4;
                    } else {
                        color &= 0x0F;
                    }
                    o.store(xpos as usize, color, color != 0, pixelattr);
                }
                rot_x += rot_a;
                rot_y += rot_c;
                xoff += 1;
                xpos += 1;
            }
        }
    }
}
