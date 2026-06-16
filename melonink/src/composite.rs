//! Scalar, bit-exact port of `GPU2D::SoftRenderer::ColorComposite` and its
//! color-effect helpers (melonDS `src/GPU2D_Soft.cpp` / `GPU2D_Soft.h`).
//!
//! All arithmetic uses wrapping ops: the C++ operates on `u32` with C
//! unsigned wraparound semantics, and some inputs really do wrap (e.g.
//! `evb = 16 - eva` when a semi-transparent sprite carries alpha > 16).
//! Matching that wraparound exactly is part of the behavior contract.

#[derive(Debug, Clone, Copy)]
pub struct Regs {
    pub blend_cnt: u32,
    pub eva: u32,
    pub evb: u32,
    pub evy: u32,
}

#[inline]
pub fn color_blend4(val1: u32, val2: u32, eva: u32, evb: u32) -> u32 {
    let mut r = (val1 & 0x00003F)
        .wrapping_mul(eva)
        .wrapping_add((val2 & 0x00003F).wrapping_mul(evb))
        .wrapping_add(0x000008)
        >> 4;
    let mut g = ((val1 & 0x003F00)
        .wrapping_mul(eva)
        .wrapping_add((val2 & 0x003F00).wrapping_mul(evb))
        .wrapping_add(0x000800)
        >> 4)
        & 0x007F00;
    let mut b = ((val1 & 0x3F0000)
        .wrapping_mul(eva)
        .wrapping_add((val2 & 0x3F0000).wrapping_mul(evb))
        .wrapping_add(0x080000)
        >> 4)
        & 0x7F0000;

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

#[inline]
pub fn color_blend5(val1: u32, val2: u32) -> u32 {
    let eva = ((val1 >> 24) & 0x1F) + 1;
    let evb = 32 - eva;

    if eva == 32 {
        return val1;
    }

    let mut r = (val1 & 0x00003F)
        .wrapping_mul(eva)
        .wrapping_add((val2 & 0x00003F).wrapping_mul(evb))
        .wrapping_add(0x000010)
        >> 5;
    let mut g = ((val1 & 0x003F00)
        .wrapping_mul(eva)
        .wrapping_add((val2 & 0x003F00).wrapping_mul(evb))
        .wrapping_add(0x001000)
        >> 5)
        & 0x007F00;
    let mut b = ((val1 & 0x3F0000)
        .wrapping_mul(eva)
        .wrapping_add((val2 & 0x3F0000).wrapping_mul(evb))
        .wrapping_add(0x100000)
        >> 5)
        & 0x7F0000;

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

#[inline]
pub fn color_brightness_up(val: u32, factor: u32, bias: u32) -> u32 {
    let rb = val & 0x3F003F;
    let g = val & 0x003F00;

    let rb = rb.wrapping_add(
        ((0x3F003Fu32.wrapping_sub(rb))
            .wrapping_mul(factor)
            .wrapping_add(bias.wrapping_mul(0x010001))
            >> 4)
            & 0x3F003F,
    );
    let g = g.wrapping_add(
        ((0x003F00u32.wrapping_sub(g))
            .wrapping_mul(factor)
            .wrapping_add(bias.wrapping_mul(0x000100))
            >> 4)
            & 0x003F00,
    );

    rb | g | 0xFF000000
}

#[inline]
pub fn color_brightness_down(val: u32, factor: u32, bias: u32) -> u32 {
    let rb = val & 0x3F003F;
    let g = val & 0x003F00;

    let rb = rb.wrapping_sub(
        (rb.wrapping_mul(factor)
            .wrapping_add(bias.wrapping_mul(0x010001))
            >> 4)
            & 0x3F003F,
    );
    let g = g.wrapping_sub(
        (g.wrapping_mul(factor)
            .wrapping_add(bias.wrapping_mul(0x000100))
            >> 4)
            & 0x003F00,
    );

    rb | g | 0xFF000000
}

/// ColorComposite(i, val1, val2), with the per-pixel window-mask byte passed
/// directly instead of indexing the member array.
#[inline]
pub fn composite_scalar(val1: u32, val2: u32, window_mask: u8, r: &Regs) -> u32 {
    let mut coloreffect = 0u32;
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
        // sprite blending
        coloreffect = 1;

        if flag1 & 0x40 != 0 {
            eva = flag1 & 0x1F;
            evb = 16u32.wrapping_sub(eva);
        } else {
            eva = r.eva;
            evb = r.evb;
        }
    } else if (flag1 & 0x40 != 0) && (blend_cnt & target2 != 0) {
        // 3D layer blending
        coloreffect = 4;
    } else {
        if flag1 & 0x80 != 0 {
            flag1 = 0x10;
        } else if flag1 & 0x40 != 0 {
            flag1 = 0x01;
        }

        if (blend_cnt & flag1 != 0) && (window_mask & 0x20 != 0) {
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
        0 => val1,
        1 => color_blend4(val1, val2, eva, evb),
        2 => color_brightness_up(val1, r.evy, 0x8),
        3 => color_brightness_down(val1, r.evy, 0x7),
        4 => color_blend5(val1, val2),
        _ => val1,
    }
}
