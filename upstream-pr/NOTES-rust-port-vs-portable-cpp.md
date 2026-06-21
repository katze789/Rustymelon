# Honest notes: which Rust-port gains are real, portable C++ optimizations

Short version: of everything in the rustymelon port, **one** change is a genuine,
portable algorithmic optimization worth upstreaming as C++ — the `BLDCNT == 0`
color-effects fast-path in this folder. The rest of the port was a faithful 1:1
translation, and its measured speedups came from things that **don't** transfer to
a C++ PR. I'd rather say that plainly than oversell a pile of "optimizations" that
wouldn't survive review.

## What was actually done in the port

- **Translation (no algorithmic change):** `TextureLookup`, `RenderPixel`, the
  `RenderPolygonScanline` span loops, all the `DrawBG_*` background modes, the
  sprite drawing, `ScanlineFinalPass`, shadow masks, `InterleaveSprites`,
  `DoCapture`. These mirror the C++ line-for-line. Porting them back to C++ would
  reproduce the existing C++ — no win.
- **Genuine algorithmic optimization:** the compositor fast-path —
  *skip the per-pixel `ColorComposite` pass when no effect is enabled.* That is
  the patch here, and it is real and portable.
- **An adaptive SIMD compositor** (AVX2) with per-chunk skips. Real speedup, but
  see the caveat below.

## Why the port's measured "gains" don't all transfer

The whole-program FPS deltas measured for the Rust core (a few %, noisy) were
mostly **not** portable optimizations:

1. **Codegen differences.** The Rust build used LTO + a single codegen unit;
   different inlining/vectorization than the stock build. That's a toolchain
   artifact, not an algorithm change.
2. **Removing a boundary we introduced.** Some "wins" were just amortizing the
   Rust↔C++ FFI call we had added (e.g. moving a per-pixel call to per-scanline).
   Pure C++ has no such boundary, so there's nothing to recover.
3. **Measurement noise.** The test laptop has ±~15% thermal variance; several
   per-function deltas were within noise.

Net: the only thing with a clear, defensible algorithmic basis is the
`BLDCNT == 0` skip.

## The SIMD compositor (optional follow-up, not in this PR)

`ColorComposite` over a scanline vectorizes well (the rustymelon version does it
8 pixels at a time with AVX2 and skips chunks with no active effect). It's a real
speedup for blend-heavy scanlines. But for melonDS it's a harder sell:

- melonDS targets **many** architectures (incl. ARM on Android/iOS), so it would
  need an `#ifdef` x86 path **plus** a NEON path or a scalar fallback — more code
  to maintain.
- The maintainers may prefer the tiny, universal scalar fast-path first.

If there's appetite, the Rust AVX2 implementation (`melonink/src/simd.rs`) is a
ready reference for a `<immintrin.h>` C++ version, and the same hash-based harness
can prove it bit-identical.

## The most valuable thing to offer upstream

Honestly, it isn't the code — it's the **behavior-preservation methodology**: a
headless harness that hashes video+audio+save-RAM per frame and does a masked
main-RAM (RetroAchievements-visible) snapshot diff between two builds, across a
game matrix with input-driven gameplay. It can prove *any* renderer optimization
(C++ or otherwise) is bit-identical to the reference. That's reusable by the
project regardless of language. See `../docs/VERIFICATION.md`.
