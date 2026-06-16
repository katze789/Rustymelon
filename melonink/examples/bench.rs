//! Microbenchmark: scalar vs AVX2 compositor on three realistic line mixes.
//! Run: cargo run --release --example bench

use melonink::composite_scalar;
use std::time::Instant;

fn xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

#[derive(Clone)]
struct Case {
    name: &'static str,
    line: [u32; 512],
    wmask: [u8; 256],
    blend_cnt: u32,
}

fn main() {
    let mut rng = 0xABCDEF123456u64;

    // Case 1: idle — no blending configured at all (menus, most 2D scenes).
    let mut idle = Case {
        name: "idle (BlendCnt=0)",
        line: [0; 512],
        wmask: [0xFF; 256],
        blend_cnt: 0,
    };
    for v in idle.line.iter_mut() {
        *v = (xorshift(&mut rng) as u32 & 0x003F3F3F) | 0x01000000;
    }

    // Case 2: configured but mostly inert — BlendCnt set, but pixel flags
    // rarely hit the targets (typical 3D gameplay scanline).
    let mut typical = Case {
        name: "typical (few hits)",
        line: [0; 512],
        wmask: [0xFF; 256],
        blend_cnt: 0x1048,
    };
    for (i, v) in typical.line.iter_mut().enumerate() {
        let flag = if i % 23 == 0 { 0x80 | 0x4A } else { 0x08 };
        *v = (xorshift(&mut rng) as u32 & 0x003F3F3F) | (flag << 24);
    }

    // Case 3: blend-heavy — semi-transparent sprites + 3D blending all over.
    let mut heavy = Case {
        name: "blend-heavy",
        line: [0; 512],
        wmask: [0xFF; 256],
        blend_cnt: 0x3FFF,
    };
    for (i, v) in heavy.line.iter_mut().enumerate() {
        let flag = match i % 3 {
            0 => 0xC0 | 0x0A, // semi-transparent sprite
            1 => 0x40,        // 3D
            _ => 0x10,
        };
        *v = (xorshift(&mut rng) as u32 & 0x003F3F3F) | (flag << 24);
    }

    const ITERS: usize = 200_000;
    let regs_of = |c: &Case| melonink_regs(c.blend_cnt, 9, 7, 11);

    for case in [&idle, &typical, &heavy] {
        // scalar
        let mut buf = case.line;
        let regs = regs_of(case);
        let t0 = Instant::now();
        for _ in 0..ITERS {
            buf.copy_from_slice(&case.line);
            for i in 0..256 {
                buf[i] = composite_scalar(buf[i], buf[256 + i], case.wmask[i], &regs);
            }
        }
        let scalar_ns = t0.elapsed().as_nanos() as f64 / ITERS as f64;

        // full entry point (dispatches to AVX2 when available)
        let mut buf2 = case.line;
        let t0 = Instant::now();
        for _ in 0..ITERS {
            buf2.copy_from_slice(&case.line);
            unsafe {
                melonink::melonink_composite_scanline(
                    buf2.as_mut_ptr(),
                    case.wmask.as_ptr(),
                    case.blend_cnt,
                    9,
                    7,
                    11,
                );
            }
        }
        let simd_ns = t0.elapsed().as_nanos() as f64 / ITERS as f64;

        assert_eq!(buf, buf2, "scalar and dispatch disagree on {}", case.name);
        println!(
            "{:20} scalar {:8.1} ns/line   simd {:8.1} ns/line   speedup {:4.2}x",
            case.name,
            scalar_ns,
            simd_ns,
            scalar_ns / simd_ns
        );
    }
}

fn melonink_regs(blend_cnt: u32, eva: u32, evb: u32, evy: u32) -> melonink::Regs {
    melonink::Regs {
        blend_cnt,
        eva,
        evb,
        evy,
    }
}
