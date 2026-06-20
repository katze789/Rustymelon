//! Bit-exact Rust port of `GPU3D::SoftRenderer::ScanlineFinalPass` and its
//! helper `CalculateFogDensity` (melonDS `src/GPU3D_Soft.cpp`).
//!
//! The final per-scanline 3D pass: edge marking, fog, and anti-aliasing, each
//! operating over the Color/Attr/Depth buffers. Called once per scanline from
//! `RenderScanline` (which stays in C++).

#[repr(C)]
pub struct FinalPassParams {
    pub color_buffer: *mut u32,
    pub depth_buffer: *const u32,
    pub attr_buffer: *mut u32,
    pub edge_table: *const u16,        // RenderEdgeTable[8]
    pub fog_density_table: *const u8,  // RenderFogDensityTable[34]
    pub buffer_size: i32,
    pub scanline_width: i32,
    pub first_pixel_offset: i32,
    pub y: i32,
    pub disp_cnt: u32,
    pub fog_color: u32,
    pub fog_offset: u32,
    pub fog_shift: u32,
}

#[inline]
fn fog_density(depth: &[u32], pixeladdr: usize, fog_offset: u32, fog_shift: u32, table: &[u8]) -> u32 {
    let mut z = depth[pixeladdr];
    let densityid;
    let densityfrac;
    if z < fog_offset {
        densityid = 0usize;
        densityfrac = 0u32;
    } else {
        z -= fog_offset;
        z = (z >> 2).wrapping_shl(fog_shift);
        let id = z >> 17;
        if id >= 32 {
            densityid = 32;
            densityfrac = 0;
        } else {
            densityid = id as usize;
            densityfrac = z & 0x1FFFF;
        }
    }
    let mut density = ((table[densityid] as u32 * (0x20000 - densityfrac))
        + (table[densityid + 1] as u32 * densityfrac))
        >> 17;
    if density >= 127 {
        density = 128;
    }
    density
}

/// # Safety
/// color/attr/depth valid for `buffer_size*2` u32 each; edge_table for 8 u16;
/// fog_density_table for 34 u8.
#[no_mangle]
pub unsafe extern "C" fn melonink_scanline_final_pass(p: *const FinalPassParams) {
    let p = &*p;
    let bsz = (p.buffer_size * 2) as usize;
    let color = core::slice::from_raw_parts_mut(p.color_buffer, bsz);
    let attr = core::slice::from_raw_parts_mut(p.attr_buffer, bsz);
    let depth = core::slice::from_raw_parts(p.depth_buffer, bsz);
    let edge_table = core::slice::from_raw_parts(p.edge_table, 8);
    let fog_table = core::slice::from_raw_parts(p.fog_density_table, 34);
    let sw = p.scanline_width as usize;
    let base = (p.first_pixel_offset + p.y * p.scanline_width) as usize;
    let buffer_size = p.buffer_size as usize;
    let disp_cnt = p.disp_cnt;

    // --- edge marking ---
    if disp_cnt & (1 << 5) != 0 {
        for x in 0..256 {
            let pixeladdr = base + x;
            let a = attr[pixeladdr];
            if a & 0xF == 0 {
                continue;
            }
            let polyid = a >> 24;
            let z = depth[pixeladdr];
            if ((polyid != (attr[pixeladdr - 1] >> 24)) && (z < depth[pixeladdr - 1]))
                || ((polyid != (attr[pixeladdr + 1] >> 24)) && (z < depth[pixeladdr + 1]))
                || ((polyid != (attr[pixeladdr - sw] >> 24)) && (z < depth[pixeladdr - sw]))
                || ((polyid != (attr[pixeladdr + sw] >> 24)) && (z < depth[pixeladdr + sw]))
            {
                let edgecolor = edge_table[(polyid >> 3) as usize] as u32;
                let mut edge_r = (edgecolor << 1) & 0x3E;
                if edge_r != 0 {
                    edge_r += 1;
                }
                let mut edge_g = (edgecolor >> 4) & 0x3E;
                if edge_g != 0 {
                    edge_g += 1;
                }
                let mut edge_b = (edgecolor >> 9) & 0x3E;
                if edge_b != 0 {
                    edge_b += 1;
                }
                color[pixeladdr] =
                    edge_r | (edge_g << 8) | (edge_b << 16) | (color[pixeladdr] & 0xFF000000);
                attr[pixeladdr] = (attr[pixeladdr] & 0xFFFFE0FF) | 0x00001000;
            }
        }
    }

    // --- fog ---
    if disp_cnt & (1 << 7) != 0 {
        let fogcolor = disp_cnt & (1 << 6) == 0;
        let mut fog_r = (p.fog_color << 1) & 0x3E;
        if fog_r != 0 {
            fog_r += 1;
        }
        let mut fog_g = (p.fog_color >> 4) & 0x3E;
        if fog_g != 0 {
            fog_g += 1;
        }
        let mut fog_b = (p.fog_color >> 9) & 0x3E;
        if fog_b != 0 {
            fog_b += 1;
        }
        let fog_a = (p.fog_color >> 16) & 0x1F;

        let blend = |color: &mut [u32], depth: &[u32], pixeladdr: usize| {
            let density = fog_density(depth, pixeladdr, p.fog_offset, p.fog_shift, fog_table);
            let srccolor = color[pixeladdr];
            let mut src_r = srccolor & 0x3F;
            let mut src_g = (srccolor >> 8) & 0x3F;
            let mut src_b = (srccolor >> 16) & 0x3F;
            let mut src_a = (srccolor >> 24) & 0x1F;
            if fogcolor {
                src_r = ((fog_r * density) + (src_r * (128 - density))) >> 7;
                src_g = ((fog_g * density) + (src_g * (128 - density))) >> 7;
                src_b = ((fog_b * density) + (src_b * (128 - density))) >> 7;
            }
            src_a = ((fog_a * density) + (src_a * (128 - density))) >> 7;
            color[pixeladdr] = src_r | (src_g << 8) | (src_b << 16) | (src_a << 24);
        };

        for x in 0..256 {
            let pixeladdr = base + x;
            let a = attr[pixeladdr];
            if a & (1 << 15) != 0 {
                blend(color, depth, pixeladdr);
            }
            // lower pixel
            if a & 0xF == 0 {
                continue;
            }
            let lower = pixeladdr + buffer_size;
            if attr[lower] & (1 << 15) == 0 {
                continue;
            }
            blend(color, depth, lower);
        }
    }

    // --- anti-aliasing ---
    if disp_cnt & (1 << 4) != 0 {
        for x in 0..256 {
            let pixeladdr = base + x;
            let a = attr[pixeladdr];
            if a & 0xF == 0 {
                continue;
            }
            let mut coverage = (a >> 8) & 0x1F;
            if coverage == 0x1F {
                continue;
            }
            if coverage == 0 {
                color[pixeladdr] = color[pixeladdr + buffer_size];
                continue;
            }
            let topcolor = color[pixeladdr];
            let mut top_r = topcolor & 0x3F;
            let mut top_g = (topcolor >> 8) & 0x3F;
            let mut top_b = (topcolor >> 16) & 0x3F;
            let mut top_a = (topcolor >> 24) & 0x1F;
            let botcolor = color[pixeladdr + buffer_size];
            let bot_r = botcolor & 0x3F;
            let bot_g = (botcolor >> 8) & 0x3F;
            let bot_b = (botcolor >> 16) & 0x3F;
            let bot_a = (botcolor >> 24) & 0x1F;
            coverage += 1;
            if bot_a > 0 {
                top_r = ((top_r * coverage) + (bot_r * (32 - coverage))) >> 5;
                top_g = ((top_g * coverage) + (bot_g * (32 - coverage))) >> 5;
                top_b = ((top_b * coverage) + (bot_b * (32 - coverage))) >> 5;
            }
            top_a = ((top_a * coverage) + (bot_a * (32 - coverage))) >> 5;
            color[pixeladdr] = top_r | (top_g << 8) | (top_b << 16) | (top_a << 24);
        }
    }
}
