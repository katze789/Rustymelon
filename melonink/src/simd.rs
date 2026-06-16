//! AVX2 batch implementation of the scanline composite. Bit-exact with
//! `composite::composite_scalar` (enforced by tests and by the game-stream
//! hash gate): every lane computes the same u32 arithmetic, with branches
//! replaced by masks. Integer adds/subs/muls wrap per lane exactly like the
//! C++ unsigned arithmetic.

#![allow(unsafe_op_in_unsafe_fn)]

use crate::composite::Regs;

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;
use core::sync::atomic::{AtomicU8, Ordering};

static AVX2_STATE: AtomicU8 = AtomicU8::new(0); // 0 unknown, 1 yes, 2 no

#[cfg(target_arch = "x86_64")]
pub fn avx2_available() -> bool {
    match AVX2_STATE.load(Ordering::Relaxed) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    let yes = detect();
    AVX2_STATE.store(if yes { 1 } else { 2 }, Ordering::Relaxed);
    yes
}

#[cfg(not(target_arch = "x86_64"))]
pub fn avx2_available() -> bool {
    false
}

#[cfg(target_arch = "x86_64")]
fn detect() -> bool {
    unsafe {
        let l1 = __cpuid(1);
        let osxsave = l1.ecx & (1 << 27) != 0;
        let avx = l1.ecx & (1 << 28) != 0;
        if !osxsave || !avx {
            return false;
        }
        // OS must save YMM state (XCR0 bits 1 and 2).
        if xcr0() & 0x6 != 0x6 {
            return false;
        }
        let l7 = __cpuid_count(7, 0);
        l7.ebx & (1 << 5) != 0 // AVX2
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "xsave")]
unsafe fn xcr0() -> u64 {
    _xgetbv(0)
}

#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn splat(x: u32) -> __m256i {
    _mm256_set1_epi32(x as i32)
}

/// All-ones lane mask where (x & m) != 0.
#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn nonzero(x: __m256i) -> __m256i {
    let z = _mm256_setzero_si256();
    let eq = _mm256_cmpeq_epi32(x, z);
    _mm256_xor_si256(eq, _mm256_set1_epi32(-1))
}

/// mask ? a : b  (mask lanes are all-ones / all-zeros)
#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn select(mask: __m256i, a: __m256i, b: __m256i) -> __m256i {
    _mm256_blendv_epi8(b, a, mask)
}

#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn blend4(v1: __m256i, v2: __m256i, eva: __m256i, evb: __m256i) -> __m256i {
    let r = _mm256_srli_epi32::<4>(_mm256_add_epi32(
        _mm256_add_epi32(
            _mm256_mullo_epi32(_mm256_and_si256(v1, splat(0x00003F)), eva),
            _mm256_mullo_epi32(_mm256_and_si256(v2, splat(0x00003F)), evb),
        ),
        splat(0x000008),
    ));
    let r = _mm256_min_epu32(r, splat(0x00003F));

    let g = _mm256_and_si256(
        _mm256_srli_epi32::<4>(_mm256_add_epi32(
            _mm256_add_epi32(
                _mm256_mullo_epi32(_mm256_and_si256(v1, splat(0x003F00)), eva),
                _mm256_mullo_epi32(_mm256_and_si256(v2, splat(0x003F00)), evb),
            ),
            splat(0x000800),
        )),
        splat(0x007F00),
    );
    let g = _mm256_min_epu32(g, splat(0x003F00));

    let b = _mm256_and_si256(
        _mm256_srli_epi32::<4>(_mm256_add_epi32(
            _mm256_add_epi32(
                _mm256_mullo_epi32(_mm256_and_si256(v1, splat(0x3F0000)), eva),
                _mm256_mullo_epi32(_mm256_and_si256(v2, splat(0x3F0000)), evb),
            ),
            splat(0x080000),
        )),
        splat(0x7F0000),
    );
    let b = _mm256_min_epu32(b, splat(0x3F0000));

    _mm256_or_si256(
        _mm256_or_si256(r, g),
        _mm256_or_si256(b, splat(0xFF000000)),
    )
}

#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn blend5(v1: __m256i, v2: __m256i) -> __m256i {
    let eva = _mm256_add_epi32(
        _mm256_and_si256(_mm256_srli_epi32::<24>(v1), splat(0x1F)),
        splat(1),
    );
    let evb = _mm256_sub_epi32(splat(32), eva);
    let m32 = _mm256_cmpeq_epi32(eva, splat(32));

    let r = _mm256_srli_epi32::<5>(_mm256_add_epi32(
        _mm256_add_epi32(
            _mm256_mullo_epi32(_mm256_and_si256(v1, splat(0x00003F)), eva),
            _mm256_mullo_epi32(_mm256_and_si256(v2, splat(0x00003F)), evb),
        ),
        splat(0x000010),
    ));
    let r = _mm256_min_epu32(r, splat(0x00003F));

    let g = _mm256_and_si256(
        _mm256_srli_epi32::<5>(_mm256_add_epi32(
            _mm256_add_epi32(
                _mm256_mullo_epi32(_mm256_and_si256(v1, splat(0x003F00)), eva),
                _mm256_mullo_epi32(_mm256_and_si256(v2, splat(0x003F00)), evb),
            ),
            splat(0x001000),
        )),
        splat(0x007F00),
    );
    let g = _mm256_min_epu32(g, splat(0x003F00));

    let b = _mm256_and_si256(
        _mm256_srli_epi32::<5>(_mm256_add_epi32(
            _mm256_add_epi32(
                _mm256_mullo_epi32(_mm256_and_si256(v1, splat(0x3F0000)), eva),
                _mm256_mullo_epi32(_mm256_and_si256(v2, splat(0x3F0000)), evb),
            ),
            splat(0x100000),
        )),
        splat(0x7F0000),
    );
    let b = _mm256_min_epu32(b, splat(0x3F0000));

    let blended = _mm256_or_si256(
        _mm256_or_si256(r, g),
        _mm256_or_si256(b, splat(0xFF000000)),
    );
    select(m32, v1, blended)
}

#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn brightness_up(v: __m256i, factor: __m256i) -> __m256i {
    let rb = _mm256_and_si256(v, splat(0x3F003F));
    let g = _mm256_and_si256(v, splat(0x003F00));

    let rb = _mm256_add_epi32(
        rb,
        _mm256_and_si256(
            _mm256_srli_epi32::<4>(_mm256_add_epi32(
                _mm256_mullo_epi32(_mm256_sub_epi32(splat(0x3F003F), rb), factor),
                splat(0x8 * 0x010001),
            )),
            splat(0x3F003F),
        ),
    );
    let g = _mm256_add_epi32(
        g,
        _mm256_and_si256(
            _mm256_srli_epi32::<4>(_mm256_add_epi32(
                _mm256_mullo_epi32(_mm256_sub_epi32(splat(0x003F00), g), factor),
                splat(0x8 * 0x000100),
            )),
            splat(0x003F00),
        ),
    );
    _mm256_or_si256(_mm256_or_si256(rb, g), splat(0xFF000000))
}

#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn brightness_down(v: __m256i, factor: __m256i) -> __m256i {
    let rb = _mm256_and_si256(v, splat(0x3F003F));
    let g = _mm256_and_si256(v, splat(0x003F00));

    let rb = _mm256_sub_epi32(
        rb,
        _mm256_and_si256(
            _mm256_srli_epi32::<4>(_mm256_add_epi32(
                _mm256_mullo_epi32(rb, factor),
                splat(0x7 * 0x010001),
            )),
            splat(0x3F003F),
        ),
    );
    let g = _mm256_sub_epi32(
        g,
        _mm256_and_si256(
            _mm256_srli_epi32::<4>(_mm256_add_epi32(
                _mm256_mullo_epi32(g, factor),
                splat(0x7 * 0x000100),
            )),
            splat(0x003F00),
        ),
    );
    _mm256_or_si256(_mm256_or_si256(rb, g), splat(0xFF000000))
}

/// Composite one 256-pixel scanline: line[0..256] = composite(line[i], line[256+i]).
///
/// # Safety
/// Caller must ensure AVX2 is available, `line.len() >= 512`, `wmask.len() >= 256`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn composite_line_avx2(line: &mut [u32], wmask: &[u8], regs: &Regs) {
    let bc = splat(regs.blend_cnt);
    let eva_r = splat(regs.eva);
    let evb_r = splat(regs.evb);
    let evy = splat(regs.evy);
    let ones = _mm256_set1_epi32(-1);
    let ce = (regs.blend_cnt >> 6) & 0x3;

    // BlendCnt == 0 kills every branch below for every pixel (no targets, no
    // effect bits): the line is provably unchanged, skip it outright.
    if regs.blend_cnt == 0 {
        return;
    }

    let top = line.as_mut_ptr();
    let bot = line.as_ptr().add(256);

    for c in 0..32 {
        let i = c * 8;
        let v1 = _mm256_loadu_si256(top.add(i) as *const __m256i);
        let v2 = _mm256_loadu_si256(bot.add(i) as *const __m256i);

        let flag1 = _mm256_srli_epi32::<24>(v1);
        let flag2 = _mm256_srli_epi32::<24>(v2);

        let m_f2_80 = nonzero(_mm256_and_si256(flag2, splat(0x80)));
        let m_f2_40 = nonzero(_mm256_and_si256(flag2, splat(0x40)));
        let target2 = select(
            m_f2_80,
            splat(0x1000),
            select(m_f2_40, splat(0x0100), _mm256_slli_epi32::<8>(flag2)),
        );
        let bct2 = nonzero(_mm256_and_si256(bc, target2));

        let m_f1_80 = nonzero(_mm256_and_si256(flag1, splat(0x80)));
        let m_f1_40 = nonzero(_mm256_and_si256(flag1, splat(0x40)));

        // Branch 1: sprite blending (blend4, per-lane eva/evb)
        let cond_a = _mm256_and_si256(m_f1_80, bct2);
        // Branch 2: 3D layer blending (blend5)
        let cond_b = _mm256_andnot_si256(cond_a, _mm256_and_si256(m_f1_40, bct2));

        let eva_sprite = _mm256_and_si256(flag1, splat(0x1F));
        let evb_sprite = _mm256_sub_epi32(splat(16), eva_sprite); // wraps like C++
        let use_sprite_alpha = _mm256_and_si256(cond_a, m_f1_40);
        let eva4 = select(use_sprite_alpha, eva_sprite, eva_r);
        let evb4 = select(use_sprite_alpha, evb_sprite, evb_r);

        // Else branch: window-gated DISPCNT effect
        let f1m = select(
            m_f1_80,
            splat(0x10),
            select(m_f1_40, splat(0x01), flag1),
        );
        let wm = _mm256_cvtepu8_epi32(_mm_loadl_epi64(wmask.as_ptr().add(i) as *const __m128i));
        let m_w = nonzero(_mm256_and_si256(wm, splat(0x20)));
        let not_ab = _mm256_andnot_si256(_mm256_or_si256(cond_a, cond_b), ones);
        let cond_w = _mm256_and_si256(
            _mm256_and_si256(nonzero(_mm256_and_si256(bc, f1m)), m_w),
            not_ab,
        );

        let mut mask4 = cond_a;
        if ce == 1 {
            mask4 = _mm256_or_si256(mask4, _mm256_and_si256(cond_w, bct2));
        }

        // Adaptive fast path: most scanline chunks have no pixel needing any
        // effect (the scalar code early-outs per pixel; we early-out per
        // chunk). Nothing to compute, nothing to store — line[i..] already
        // holds val1.
        let effect_w = ce == 2 || ce == 3;
        let any_work = if effect_w {
            _mm256_or_si256(_mm256_or_si256(mask4, cond_b), cond_w)
        } else {
            _mm256_or_si256(mask4, cond_b)
        };
        if _mm256_testz_si256(any_work, any_work) != 0 {
            continue;
        }

        let mut res = v1;
        if _mm256_testz_si256(cond_b, cond_b) == 0 {
            res = select(cond_b, blend5(v1, v2), res);
        }
        if _mm256_testz_si256(mask4, mask4) == 0 {
            res = select(mask4, blend4(v1, v2, eva4, evb4), res);
        }
        if ce == 2 {
            if _mm256_testz_si256(cond_w, cond_w) == 0 {
                res = select(cond_w, brightness_up(v1, evy), res);
            }
        } else if ce == 3 {
            if _mm256_testz_si256(cond_w, cond_w) == 0 {
                res = select(cond_w, brightness_down(v1, evy), res);
            }
        }

        _mm256_storeu_si256(top.add(i) as *mut __m256i, res);
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn composite_line_avx2(_line: &mut [u32], _wmask: &[u8], _regs: &Regs) {
    unreachable!("avx2_available() is always false off x86_64");
}
