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
  - GPU2D `DrawBG_Affine` + `DrawBG_Extended` + `DrawBG_Large` — affine/extended/
    large background modes.
  - GPU2D `InterleaveSprites`, `DrawBG_3D`, `ApplySpriteMosaicX`, `DoCapture`.
  - GPU3D `ScanlineFinalPass` (edge-marking + fog + anti-aliasing) +
    `CalculateFogDensity`, and `RenderShadowMaskScanline`.
  - **With these, the *complete* software-renderer pixel pipeline is in Rust** —
    all 2D BG/sprite drawing, all 3D rasterization, the final pass, shadow masks,
    compositing, and display capture.
- **Production verification gate.** `verify-all.ps1` runs unit/fuzz tests +
  stream parity + RA-RAM proof in one command and exits non-zero on any failure.
- **Distinct fork identity.** The build reports `library_name = "rustymelon DS"`
  (guarded by `#ifdef MELONINK`), so it cannot be mistaken for the approved
  `melonDS DS` core and RetroAchievements Hardcore correctly refuses it. The
  identity change is a separate small patch (`patches/melonds-ds-rustymelon.patch`).
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
| `DrawBG_Affine` / `_Extended` / `_Large` | small | small | Rust ✓ |
| `RenderPolygonScanline` (per-scanline setup) | 3.2 % | 4.0 % | C++ (kept) |
| `RenderThreadFunc` / `register_frame_ctor` | ~10 % | ~9 % | threading/overhead — not productively portable |
| `SPUChannel::Run` | — | 3.3 % | C++ audio (sensitive; out of scope) |
| ARM JIT `Execute` | (in "other" ~73 %) | | **off-limits** (CPU/timing → RA risk) |

**The complete software-renderer pixel pipeline is now in Rust** — every per-pixel
drawing/compositing path, including the final pass, shadow masks, and display
capture. What remains in C++ is, by design: the once-per-scanline geometry setup
(slope/edge/Y-interpolation), the scanline orchestration/dispatch, render-thread
management, audio, and the ARM JIT (off-limits — CPU/timing). All ports were
validated with input-driven gameplay tests (e.g. the MKDS in-race affine minimap)
on top of the usual gate (see ACCURACY.md).

## Next (optional; diminishing returns)

The software-renderer port is complete. Remaining ideas, all optional:

1. **Broaden coverage of the rarely-hit paths.** Some functions pass wherever the
   8-game matrix + input-driven tests exercise them, but are not heavily covered:
   the extended/large *bitmap* BG modes, and `DoCapture` (display capture — used
   by e.g. battle-transition effects). Scripted inputs into capture-heavy games
   would tighten this. The Rust transcriptions mirror the C++ exactly regardless.
2. **Widen the differential net.** A true C++-oracle differential for the ported
   functions (compile the upstream function standalone and fuzz Rust against it),
   beyond the current independent-reference fuzz + real-game gate.
3. **Upstream / C++ reimplementation.** See RETROACHIEVEMENTS.md — the path with a
   real chance of a legitimately-approved, shipping result.

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
