# Roadmap

Accuracy-first, local-only. Each step ships only when it passes the full
verification (`VERIFICATION.md`): unit/fuzz tests, whole-core stream parity on
the game matrix, and the RA-RAM proof where the ROM permits it.

## Done

- **Harness (`melonbench`, all Rust).** Headless libretro runner with FPS /
  frame-time percentiles, a sampling profiler, a suite runner with per-game
  determinism flags, a symbolizer, and `ramverify` (multi-ref masked RAM diff).
- **Profiling.** Hot spots identified: 3D software rasterizer
  (`RenderPolygonScanline` + `TextureLookup`, 33–56 %), 2D compositor (15–24 %),
  CPU/JIT (~5–12 %).
- **`melonink` ports (Rust, `no_std`):**
  - GPU2D `ColorComposite` scanline loop — AVX2 + scalar, exhaustively tested.
  - GPU3D `TextureLookup` — all 8 formats, 3M-case fuzz vs independent reference.
  - GPU3D `RenderPixel` — pure per-pixel shader, 2M-case fuzz.
  - **`RenderPolygonScanline` per-pixel span loop** — the dominant rasterizer
    cost, ported at scanline granularity (one FFI call per scanline). Includes
    `Interpolator<0>`, the four depth tests, `AlphaBlend`, `PlotTranslucentPixel`.
  - GPU2D `DrawBG_Text` pixel loops — 256/16-colour text BG (mosaic + ext-pal).
  - GPU2D `DrawSprite_Normal` + `DrawSprite_Rotscale` — full sprite rendering
    (bitmap / 256 / 16-colour, affine, window, flips, mosaic).
- **Verification infrastructure.** Video+audio+save-RAM stream gate; RA-visible
  main-RAM proof via masked snapshots; input-driven gameplay tests; documented
  ROM-determinism caveats.
- **Accuracy:** 8-game matrix all pass the stream gate; RA-RAM proof passes on
  every RAM-deterministic ROM (see `ACCURACY.md`). Every port above landed
  bit-identical.
- **Performance:** all four 3D games improved by the span-loop port
  (CTGP **+16.6 %** with fewer spikes; SM64DS-NB +10 %, MKDS +6.9 %, SM64DS
  +3.1 %); the `DrawBG_Text` port additionally nudged 2D games up
  (Pokémon +9.5 %, NSMB +4.5 %) with no regressions (see `ACCURACY.md`).

## Hot-spot profile (after the renderer port)

Re-profiled the Rust core (sampling, 3000 frames). **In-DLL rendering is only
~27 % of total time for CTGP** — the other ~73 % is the ARM JIT executing game
code plus system/driver, which is off-limits (CPU/timing → RA). Within the
renderer, the hot path is now Rust:

| Function | CTGP (3D) | Pokémon (2D/RPG) | Status |
|---|---|---|---|
| `melonink_render_spans` | 14.6 % | 12.9 % | Rust ✓ |
| `texture_lookup` | 7.0 % | 5.6 % | Rust ✓ |
| `render_pixel` | 4.6 % | 5.2 % | Rust ✓ |
| `DrawBG_Text` | 4.9 % | 5.0 % | Rust ✓ |
| `DrawSprite_Normal` / `_Rotscale` | 5.3 % | 5.5 % | Rust ✓ |
| `RenderPolygonScanline` (per-scanline setup) | 3.2 % | 4.0 % | C++ (kept) |
| `RenderThreadFunc` / `register_frame_ctor` | ~10 % | ~9 % | threading/overhead — not productively portable |
| `SPUChannel::Run` | — | 3.3 % | C++ audio (sensitive; out of scope) |
| ARM JIT `Execute` | (in "other" ~73 %) | | **off-limits** (CPU/timing → RA risk) |

**Every prominent renderer hot path is now in Rust.** What remains in C++ is the
once-per-scanline setup (kept by design), threading/overhead that can't be
meaningfully ported, audio, and the ARM JIT (off-limits). The sprite port —
including the affine `DrawSprite_Rotscale` path — was validated with input-driven
gameplay tests on top of the usual gate (see ACCURACY.md).

## Next (optional; diminishing returns)

1. **Widen the differential net.** A true C++-oracle differential for
   `TextureLookup`/`RenderPixel`/sprites (compile the upstream function standalone
   and fuzz Rust against it), beyond the current independent-reference fuzz +
   real-game gate.

2. **Broaden game/input coverage further.** More titles and scripted input paths
   that reach varied in-game states (the sprite port showed intros under-cover
   some modes). The minor remaining 2D-BG variants (`DrawBG_Affine`,
   `DrawBG_Extended`, `DrawBG_Large`) could be ported too, but they are rare and
   low-value.

3. **Upstream the patch** (see RETROACHIEVEMENTS.md) — the highest-value next
   step if the goal is an approved, shipping optimisation.

### Investigated and deliberately set aside

- **SIMD-ing the span loop.** Its per-pixel work includes a perspective-correct
  integer division (no AVX2 op) and a VRAM gather in `texture_lookup`, so only a
  small fraction is batchable and bit-exactness would be very hard. The accuracy
  risk far outweighs a sub-1 % gain. (If ever revisited, it must use the same
  scalar-vs-SIMD differential discipline as the compositor.)

## Not doing (unless explicitly decided)

- Publishing anything (`LOCAL-ONLY.md`).
- Spoofing the core identity to pass RA Hardcore. The only legitimate route to
  Hardcore is upstreaming into official melonDS so the approved build carries
  the change — a separate, explicit decision for later.
- Touching scheduler/CPU/GPU *timing*. Performance work stays behaviour-
  preserving and hash-gated.
