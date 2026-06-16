//! Bit-exact Rust port of the three per-pixel span loops at the end of
//! `GPU3D::SoftRenderer::RenderPolygonScanline` (melonDS `src/GPU3D_Soft.cpp`),
//! plus the helpers they call: the X-axis `Interpolator`, the four depth-test
//! modes, `AlphaBlend`, and `PlotTranslucentPixel`.
//!
//! The once-per-scanline setup (slope/edge setup, Y-interpolation of the span
//! endpoints) stays in C++; this function runs the hot per-pixel work so the
//! language boundary is crossed once per scanline instead of once per pixel.
//!
//! Every integer operation mirrors the C++ promotion/width semantics exactly
//! (verified by the whole-core hash gate + ramverify). `render_pixel` is the
//! already-ported pure shader; it is called directly here (no FFI hop).

use crate::renderpixel::render_pixel;
use crate::texture::Vram;

/// X-axis interpolator (the `Interpolator<0>` specialisation, `dir == 0`).
struct InterpX {
    x0: i32,
    xdiff: i32,
    x: i32,
    linear: bool,
    xrecip_z: i32,
    w0n: i32,
    w0d: i32,
    w1d: i32,
    yfactor: u32,
}

impl InterpX {
    const SHIFT: u32 = 8; // dir == 0

    fn new(x0: i32, x1: i32, w0: i32, w1: i32) -> InterpX {
        let xdiff = x1 - x0;
        let xrecip_z = if xdiff != 0 { (1 << 22) / xdiff } else { 0 };
        // dir==0: mask 0x7F
        let linear = (w0 == w1) && (w0 & 0x7F == 0) && (w1 & 0x7F == 0);
        InterpX {
            x0,
            xdiff,
            x: 0,
            linear,
            xrecip_z,
            w0n: w0,
            w0d: w0,
            w1d: w1,
            yfactor: 0,
        }
    }

    fn set_x(&mut self, x: i32) {
        let x = x - self.x0;
        self.x = x;
        if self.xdiff != 0 && !self.linear {
            // num is s64 (no overflow); den is s32 and may overflow on large W
            // — C++ relies on the wraparound, so replicate it explicitly.
            let num = ((x as i64) * (self.w0n as i64)) << Self::SHIFT;
            let den = x
                .wrapping_mul(self.w0d)
                .wrapping_add((self.xdiff - x).wrapping_mul(self.w1d));
            self.yfactor = if den == 0 { 0 } else { (num / den as i64) as u32 };
        }
    }

    fn interpolate(&self, y0: i32, y1: i32) -> i32 {
        if self.xdiff == 0 || y0 == y1 {
            return y0;
        }
        if !self.linear {
            if y0 < y1 {
                y0.wrapping_add(
                    (((y1 - y0) as u32).wrapping_mul(self.yfactor) >> Self::SHIFT) as i32,
                )
            } else {
                y1.wrapping_add(
                    (((y0 - y1) as u32)
                        .wrapping_mul((1u32 << Self::SHIFT).wrapping_sub(self.yfactor))
                        >> Self::SHIFT) as i32,
                )
            }
        } else if y0 < y1 {
            y0 + ((((y1 - y0) as i64) * (self.x as i64) / (self.xdiff as i64)) as i32)
        } else {
            y1 + ((((y0 - y1) as i64) * ((self.xdiff - self.x) as i64) / (self.xdiff as i64))
                as i32)
        }
    }

    fn interpolate_z(&self, z0: i32, z1: i32, wbuffer: bool) -> i32 {
        if self.xdiff == 0 || z0 == z1 {
            return z0;
        }
        if wbuffer {
            if z0 < z1 {
                z0.wrapping_add(
                    ((((z1 - z0) as i64) * (self.yfactor as i64)) >> Self::SHIFT) as i32,
                )
            } else {
                // (256 - yfactor) as u32 then zero-extended, matching C++.
                let f = (1u32 << Self::SHIFT).wrapping_sub(self.yfactor) as i64;
                z1.wrapping_add(((((z0 - z1) as i64) * f) >> Self::SHIFT) as i32)
            }
        } else {
            // Z-buffering, dir == 0
            let (base, mut disp, factor) = if z0 < z1 {
                (z0, z1 - z0, self.x)
            } else {
                (z1, z0 - z1, self.xdiff - self.x)
            };
            disp >>= 9;
            base + (((disp as i64) * (factor as i64) * (self.xrecip_z as i64)) >> 13) as i32
        }
    }
}

#[inline]
fn depth_test(mode: u8, dstz: i32, z: i32, dstattr: u32) -> bool {
    match mode {
        0 => {
            // Equal_Z
            let diff = dstz.wrapping_sub(z);
            (diff.wrapping_add(0x200) as u32) <= 0x400
        }
        1 => {
            // Equal_W
            let diff = dstz.wrapping_sub(z);
            (diff.wrapping_add(0xFF) as u32) <= 0x1FE
        }
        2 => z < dstz, // LessThan
        _ => {
            // LessThan_FrontFacing
            if (dstattr & 0x00400010) == 0x00000010 {
                z <= dstz
            } else {
                z < dstz
            }
        }
    }
}

#[inline]
fn alpha_blend(disp_cnt: u32, srccolor: u32, dstcolor: u32, alpha: u32) -> u32 {
    let mut dstalpha = dstcolor >> 24;
    if dstalpha == 0 {
        return srccolor;
    }
    let mut src_r = srccolor & 0x3F;
    let mut src_g = (srccolor >> 8) & 0x3F;
    let mut src_b = (srccolor >> 16) & 0x3F;

    if disp_cnt & (1 << 3) != 0 {
        let dst_r = dstcolor & 0x3F;
        let dst_g = (dstcolor >> 8) & 0x3F;
        let dst_b = (dstcolor >> 16) & 0x3F;
        let a = alpha + 1;
        src_r = ((src_r * a) + (dst_r * (32 - a))) >> 5;
        src_g = ((src_g * a) + (dst_g * (32 - a))) >> 5;
        src_b = ((src_b * a) + (dst_b * (32 - a))) >> 5;
    }

    if alpha > dstalpha {
        dstalpha = alpha;
    }
    src_r | (src_g << 8) | (src_b << 16) | (dstalpha << 24)
}

/// Mutable buffer views the loops read and write.
struct Buffers<'a> {
    color: &'a mut [u32],
    depth: &'a mut [u32],
    attr: &'a mut [u32],
    stencil: &'a [u8],
}

#[allow(clippy::too_many_arguments)]
fn plot_translucent_pixel(
    bufs: &mut Buffers,
    buffer_size: usize,
    disp_cnt: u32,
    pixeladdr: usize,
    color: u32,
    z: i32,
    polyattr: u32,
    shadow: bool,
) {
    let dstattr = bufs.attr[pixeladdr];
    let mut attr =
        (polyattr & 0xE0F0) | ((polyattr >> 8) & 0xFF0000) | (1 << 22) | (dstattr & 0xFF001F0F);

    if shadow {
        if dstattr & (1 << 22) != 0 {
            if (dstattr & 0x007F0000) == (attr & 0x007F0000) {
                return;
            }
        } else if (dstattr & 0x3F000000) == (polyattr & 0x3F000000) {
            return;
        }
    } else if (dstattr & 0x007F0000) == (attr & 0x007F0000) {
        return;
    }

    if dstattr & (1 << 15) == 0 {
        attr &= !(1 << 15);
    }

    let blended = alpha_blend(disp_cnt, color, bufs.color[pixeladdr], color >> 24);

    if z != -1 {
        bufs.depth[pixeladdr] = z as u32;
    }
    bufs.color[pixeladdr] = blended;
    bufs.attr[pixeladdr] = attr;
    let _ = buffer_size;
}

/// Parameters for one scanline span, mirroring the locals the C++ has computed
/// by the time it reaches "part 1: left edge".
#[repr(C)]
pub struct SpanParams {
    pub color_buffer: *mut u32,
    pub depth_buffer: *mut u32,
    pub attr_buffer: *mut u32,
    pub stencil_buffer: *const u8,
    pub tex: *const u8,
    pub pal: *const u8,
    pub toon_table: *const u16,

    pub buffer_size: i32,
    pub scanline_width: i32,
    pub first_pixel_offset: i32,
    pub y: i32,

    pub xstart: i32,
    pub xend: i32,
    pub wl: i32,
    pub wr: i32,
    pub zl: i32,
    pub zr: i32,
    pub rl: i32,
    pub gl: i32,
    pub bl: i32,
    pub sl: i32,
    pub tl: i32,
    pub rr: i32,
    pub gr: i32,
    pub br: i32,
    pub sr: i32,
    pub tr: i32,
    pub l_edgelen: i32,
    pub r_edgelen: i32,
    pub l_edgecov: i32,
    pub r_edgecov: i32,
    pub yedge: i32,

    pub l_filledge: u8,
    pub r_filledge: u8,
    pub wireframe: u8,
    pub is_shadow: u8,
    pub wbuffer: u8,
    pub depth_test_mode: u8,
    pub _pad0: u8,
    pub _pad1: u8,

    pub poly_attr: u32,
    pub poly_attr_full: u32,
    pub tex_param: u32,
    pub tex_palette: u32,
    pub disp_cnt: u32,
    pub alpha_ref: u32,
}

/// # Safety
/// All pointers in `*p` must be valid for the documented lengths and the scalar
/// fields must describe a single scanline span exactly as the C++ computed them.
#[no_mangle]
pub unsafe extern "C" fn melonink_render_spans(p: *const SpanParams) {
    let p = &*p;
    let bsz = p.buffer_size as usize;
    let mut bufs = Buffers {
        color: core::slice::from_raw_parts_mut(p.color_buffer, bsz * 2),
        depth: core::slice::from_raw_parts_mut(p.depth_buffer, bsz * 2),
        attr: core::slice::from_raw_parts_mut(p.attr_buffer, bsz * 2),
        stencil: core::slice::from_raw_parts(p.stencil_buffer, 512),
    };
    let vram = Vram {
        tex: core::slice::from_raw_parts(p.tex, 512 * 1024),
        pal: core::slice::from_raw_parts(p.pal, 128 * 1024),
    };
    let toon = &*(p.toon_table as *const [u16; 32]);

    let buffer_size = bsz;
    let scanline_base = (p.first_pixel_offset + p.y * p.scanline_width) as usize;
    let wbuffer = p.wbuffer != 0;
    let is_shadow = p.is_shadow != 0;
    let wireframe = p.wireframe != 0;
    let dtmode = p.depth_test_mode;

    let mut interp = InterpX::new(p.xstart, p.xend + 1, p.wl, p.wr);

    // Shared across the three parts (C++ keeps x in a single variable).
    let mut x = p.xstart;
    if x < 0 {
        x = 0;
    }

    // Closure-free helper to shade + store one pixel given the precomputed edge
    // attr bits. Returns nothing; mutates buffers.
    // (Inlined manually per part to match the slightly different AA handling.)

    // ---- part 1: left edge ----
    let mut edge = p.yedge | 0x1;
    let mut xlimit = p.xstart + p.l_edgelen;
    if xlimit > p.xend + 1 {
        xlimit = p.xend + 1;
    }
    if xlimit > 256 {
        xlimit = 256;
    }
    let mut xcov: i32 = 0;
    if p.l_edgecov & (1 << 31) != 0 {
        xcov = (p.l_edgecov >> 12) & 0x3FF;
        if xcov == 0x3FF {
            xcov = 0;
        }
    }

    if p.l_filledge == 0 {
        x = xlimit;
    } else {
        while x < xlimit {
            let mut pixeladdr = scanline_base + x as usize;
            let mut dstattr = bufs.attr[pixeladdr];

            if is_shadow {
                let stencil = bufs.stencil[256 * ((p.y & 0x1) as usize) + x as usize];
                if stencil == 0 {
                    x += 1;
                    continue;
                }
                if stencil & 0x1 == 0 {
                    pixeladdr += buffer_size;
                }
                if stencil & 0x2 == 0 {
                    dstattr &= !0xF;
                }
            }

            interp.set_x(x);
            let z = interp.interpolate_z(p.zl, p.zr, wbuffer);

            if !depth_test(dtmode, bufs.depth[pixeladdr] as i32, z, dstattr) {
                if (dstattr & 0xF) == 0 || pixeladdr >= buffer_size {
                    x += 1;
                    continue;
                }
                pixeladdr += buffer_size;
                dstattr = bufs.attr[pixeladdr];
                if !depth_test(dtmode, bufs.depth[pixeladdr] as i32, z, dstattr) {
                    x += 1;
                    continue;
                }
            }

            let vr = interp.interpolate(p.rl, p.rr) as u32;
            let vg = interp.interpolate(p.gl, p.gr) as u32;
            let vb = interp.interpolate(p.bl, p.br) as u32;
            let s = interp.interpolate(p.sl, p.sr) as i16;
            let t = interp.interpolate(p.tl, p.tr) as i16;

            let color = render_pixel(
                &vram,
                p.poly_attr_full,
                p.tex_param,
                p.tex_palette,
                p.disp_cnt,
                toon,
                (vr >> 3) as u8,
                (vg >> 3) as u8,
                (vb >> 3) as u8,
                s,
                t,
            );
            let alpha = color >> 24;

            if alpha <= p.alpha_ref {
                x += 1;
                continue;
            }

            if alpha == 31 {
                let mut attr = p.poly_attr | edge as u32;
                if p.disp_cnt & (1 << 4) != 0 {
                    let mut cov = p.l_edgecov;
                    if cov & (1 << 31) != 0 {
                        cov = xcov >> 5;
                        if cov > 31 {
                            cov = 31;
                        }
                        xcov += p.l_edgecov & 0x3FF;
                    }
                    attr |= (cov as u32) << 8;
                    if pixeladdr < buffer_size {
                        bufs.color[pixeladdr + buffer_size] = bufs.color[pixeladdr];
                        bufs.depth[pixeladdr + buffer_size] = bufs.depth[pixeladdr];
                        bufs.attr[pixeladdr + buffer_size] = bufs.attr[pixeladdr];
                    }
                }
                bufs.depth[pixeladdr] = z as u32;
                bufs.color[pixeladdr] = color;
                bufs.attr[pixeladdr] = attr;
            } else {
                let mut zz = z;
                if p.poly_attr_full & (1 << 11) == 0 {
                    zz = -1;
                }
                plot_translucent_pixel(
                    &mut bufs, buffer_size, p.disp_cnt, pixeladdr, color, zz, p.poly_attr,
                    is_shadow,
                );
                if (dstattr & 0xF) != 0 && pixeladdr < buffer_size {
                    plot_translucent_pixel(
                        &mut bufs,
                        buffer_size,
                        p.disp_cnt,
                        pixeladdr + buffer_size,
                        color,
                        zz,
                        p.poly_attr,
                        is_shadow,
                    );
                }
            }
            x += 1;
        }
    }

    // ---- part 2: polygon inside ----
    edge = p.yedge;
    xlimit = p.xend - p.r_edgelen + 1;
    if xlimit > p.xend + 1 {
        xlimit = p.xend + 1;
    }
    if xlimit > 256 {
        xlimit = 256;
    }

    if wireframe && edge == 0 {
        if x < xlimit {
            x = xlimit;
        }
    } else {
        while x < xlimit {
            let mut pixeladdr = scanline_base + x as usize;
            let mut dstattr = bufs.attr[pixeladdr];

            if is_shadow {
                let stencil = bufs.stencil[256 * ((p.y & 0x1) as usize) + x as usize];
                if stencil == 0 {
                    x += 1;
                    continue;
                }
                if stencil & 0x1 == 0 {
                    pixeladdr += buffer_size;
                }
                if stencil & 0x2 == 0 {
                    dstattr &= !0xF;
                }
            }

            interp.set_x(x);
            let z = interp.interpolate_z(p.zl, p.zr, wbuffer);

            if !depth_test(dtmode, bufs.depth[pixeladdr] as i32, z, dstattr) {
                if (dstattr & 0xF) == 0 || pixeladdr >= buffer_size {
                    x += 1;
                    continue;
                }
                pixeladdr += buffer_size;
                dstattr = bufs.attr[pixeladdr];
                if !depth_test(dtmode, bufs.depth[pixeladdr] as i32, z, dstattr) {
                    x += 1;
                    continue;
                }
            }

            let vr = interp.interpolate(p.rl, p.rr) as u32;
            let vg = interp.interpolate(p.gl, p.gr) as u32;
            let vb = interp.interpolate(p.bl, p.br) as u32;
            let s = interp.interpolate(p.sl, p.sr) as i16;
            let t = interp.interpolate(p.tl, p.tr) as i16;

            let color = render_pixel(
                &vram,
                p.poly_attr_full,
                p.tex_param,
                p.tex_palette,
                p.disp_cnt,
                toon,
                (vr >> 3) as u8,
                (vg >> 3) as u8,
                (vb >> 3) as u8,
                s,
                t,
            );
            let alpha = color >> 24;

            if alpha <= p.alpha_ref {
                x += 1;
                continue;
            }

            if alpha == 31 {
                let mut attr = p.poly_attr | edge as u32;
                if (p.disp_cnt & (1 << 4) != 0) && (attr & 0xF != 0) {
                    attr |= 0x1F << 8;
                    if pixeladdr < buffer_size {
                        bufs.color[pixeladdr + buffer_size] = bufs.color[pixeladdr];
                        bufs.depth[pixeladdr + buffer_size] = bufs.depth[pixeladdr];
                        bufs.attr[pixeladdr + buffer_size] = bufs.attr[pixeladdr];
                    }
                }
                bufs.depth[pixeladdr] = z as u32;
                bufs.color[pixeladdr] = color;
                bufs.attr[pixeladdr] = attr;
            } else {
                let mut zz = z;
                if p.poly_attr_full & (1 << 11) == 0 {
                    zz = -1;
                }
                plot_translucent_pixel(
                    &mut bufs, buffer_size, p.disp_cnt, pixeladdr, color, zz, p.poly_attr,
                    is_shadow,
                );
                if (dstattr & 0xF) != 0 && pixeladdr < buffer_size {
                    plot_translucent_pixel(
                        &mut bufs,
                        buffer_size,
                        p.disp_cnt,
                        pixeladdr + buffer_size,
                        color,
                        zz,
                        p.poly_attr,
                        is_shadow,
                    );
                }
            }
            x += 1;
        }
    }

    // ---- part 3: right edge ----
    edge = p.yedge | 0x2;
    xlimit = p.xend + 1;
    if xlimit > 256 {
        xlimit = 256;
    }
    xcov = 0;
    if p.r_edgecov & (1 << 31) != 0 {
        xcov = (p.r_edgecov >> 12) & 0x3FF;
        if xcov == 0x3FF {
            xcov = 0;
        }
    }

    if p.r_filledge != 0 {
        while x < xlimit {
            let mut pixeladdr = scanline_base + x as usize;
            let mut dstattr = bufs.attr[pixeladdr];

            if is_shadow {
                let stencil = bufs.stencil[256 * ((p.y & 0x1) as usize) + x as usize];
                if stencil == 0 {
                    x += 1;
                    continue;
                }
                if stencil & 0x1 == 0 {
                    pixeladdr += buffer_size;
                }
                if stencil & 0x2 == 0 {
                    dstattr &= !0xF;
                }
            }

            interp.set_x(x);
            let z = interp.interpolate_z(p.zl, p.zr, wbuffer);

            if !depth_test(dtmode, bufs.depth[pixeladdr] as i32, z, dstattr) {
                if (dstattr & 0xF) == 0 || pixeladdr >= buffer_size {
                    x += 1;
                    continue;
                }
                pixeladdr += buffer_size;
                dstattr = bufs.attr[pixeladdr];
                if !depth_test(dtmode, bufs.depth[pixeladdr] as i32, z, dstattr) {
                    x += 1;
                    continue;
                }
            }

            let vr = interp.interpolate(p.rl, p.rr) as u32;
            let vg = interp.interpolate(p.gl, p.gr) as u32;
            let vb = interp.interpolate(p.bl, p.br) as u32;
            let s = interp.interpolate(p.sl, p.sr) as i16;
            let t = interp.interpolate(p.tl, p.tr) as i16;

            let color = render_pixel(
                &vram,
                p.poly_attr_full,
                p.tex_param,
                p.tex_palette,
                p.disp_cnt,
                toon,
                (vr >> 3) as u8,
                (vg >> 3) as u8,
                (vb >> 3) as u8,
                s,
                t,
            );
            let alpha = color >> 24;

            if alpha <= p.alpha_ref {
                x += 1;
                continue;
            }

            if alpha == 31 {
                let mut attr = p.poly_attr | edge as u32;
                if p.disp_cnt & (1 << 4) != 0 {
                    let mut cov = p.r_edgecov;
                    if cov & (1 << 31) != 0 {
                        cov = 0x1F - (xcov >> 5);
                        if cov < 0 {
                            cov = 0;
                        }
                        xcov += p.r_edgecov & 0x3FF;
                    }
                    attr |= (cov as u32) << 8;
                    if pixeladdr < buffer_size {
                        bufs.color[pixeladdr + buffer_size] = bufs.color[pixeladdr];
                        bufs.depth[pixeladdr + buffer_size] = bufs.depth[pixeladdr];
                        bufs.attr[pixeladdr + buffer_size] = bufs.attr[pixeladdr];
                    }
                }
                bufs.depth[pixeladdr] = z as u32;
                bufs.color[pixeladdr] = color;
                bufs.attr[pixeladdr] = attr;
            } else {
                let mut zz = z;
                if p.poly_attr_full & (1 << 11) == 0 {
                    zz = -1;
                }
                plot_translucent_pixel(
                    &mut bufs, buffer_size, p.disp_cnt, pixeladdr, color, zz, p.poly_attr,
                    is_shadow,
                );
                if (dstattr & 0xF) != 0 && pixeladdr < buffer_size {
                    plot_translucent_pixel(
                        &mut bufs,
                        buffer_size,
                        p.disp_cnt,
                        pixeladdr + buffer_size,
                        color,
                        zz,
                        p.poly_attr,
                        is_shadow,
                    );
                }
            }
            x += 1;
        }
    }
}
