# rustymelon

**Rust-accelerated, accuracy-first modifications to the melonDS DS libretro core.**

rustymelon moves the hottest, most bug-prone hot paths of the melonDS Nintendo DS
software renderer from C++ into Rust, with one non-negotiable rule: **output must
stay bit-identical to upstream melonDS.** The goal is better performance on weak
hardware *and* memory-safer code, without changing a single thing a game — or
[RetroAchievements](https://retroachievements.org/) — can observe.

This is **not** a fork of the whole emulator. It is a small set of Rust modules
(`melonink`) plus a tiny, additive patch to melonDS, all guarded behind
`#ifdef MELONINK` so the same tree builds byte-for-byte upstream when the flag is
off.

> [!IMPORTANT]
> **Custom build — not RetroAchievements-approved.** rustymelon reports its **own
> distinct core identity** (`rustymelon DS`), specifically so it can never be
> mistaken for — or accepted in place of — the approved `melonDS DS` core. That
> means RetroAchievements Hardcore will (correctly) not accept it. It installs
> *alongside* the stock core, never replacing it. Use the stock core for anything
> that counts. See [docs/RETROACHIEVEMENTS.md](docs/RETROACHIEVEMENTS.md).

## Status

Ported to Rust so far (each bit-identical to upstream, verified — see below):

| Area | Upstream function | 
|---|---|
| 2D colour-effect compositor | `GPU2D::SoftRenderer::ColorComposite` scanline loop (AVX2 + scalar) |
| 2D text backgrounds | `GPU2D::SoftRenderer::DrawBG_Text` pixel loops |
| 2D sprites | `GPU2D::SoftRenderer::DrawSprite_Normal` + `DrawSprite_Rotscale` (bitmap / 256 / 16-colour, affine, window, flips, mosaic) |
| 2D affine/extended/large backgrounds | `GPU2D::SoftRenderer::DrawBG_Affine` + `DrawBG_Extended` + `DrawBG_Large` |
| 3D texture sampling | `GPU3D::SoftRenderer::TextureLookup` (all 8 NDS formats) |
| 3D pixel shading | `GPU3D::SoftRenderer::RenderPixel` |
| 3D rasterizer span loop | `RenderPolygonScanline` per-pixel loops + interpolator, depth tests, alpha blend, translucent plot |
| 3D final pass | `ScanlineFinalPass` (edge-marking, fog, anti-aliasing) + `CalculateFogDensity` |
| 3D shadow masks | `RenderShadowMaskScanline` |
| sprite/3D compositing | `InterleaveSprites`, `DrawBG_3D`, `ApplySpriteMosaicX` |
| display capture | `DoCapture` |

Also ported: `InterleaveSprites`, `DrawBG_3D`, `ApplySpriteMosaicX`, the 3D
`ScanlineFinalPass` (edge-marking, fog, anti-aliasing), `RenderShadowMaskScanline`,
and `DoCapture`.

**The complete software-renderer pixel pipeline is now in Rust.** What remains in
C++ is, by design: the per-scanline geometry setup (slope/edge setup,
Y-interpolation), the scanline orchestration/dispatch, the render-thread
management, and everything outside the renderer — the ARM CPU JIT, audio (SPU),
scheduler, DMA, timers, memory — which is deliberately untouched because changing
CPU/timing would break accuracy and RetroAchievements.

**Performance** (median, stable clocks, on an i5-8250U laptop): all test games
improve, with notably smoother frame pacing — e.g. MKDS +13.5 % (P99 frame time
20.5→12.8 ms), SM64DS +10.7 %, CTGP Nitro +16.6 %. Full numbers and methodology
in [docs/ACCURACY.md](docs/ACCURACY.md).

**Why not more?** ~73 % of runtime is the ARM CPU JIT executing game code, which
is deliberately untouched (changing CPU/timing would break accuracy and
RetroAchievements). The renderer is the remaining ~27 %, and its prominent hot
paths are now all in Rust. See [docs/ROADMAP.md](docs/ROADMAP.md).

## Accuracy is the whole point

Every change is proven behaviour-preserving by four independent layers
(full detail in [docs/VERIFICATION.md](docs/VERIFICATION.md)):

1. **Unit / differential fuzz tests** in `melonink` — each ported function vs an
   independently-written reference over millions of random inputs (e.g. 3 M cases
   for `TextureLookup`, exhaustive flag coverage for the compositor).
2. **Whole-core stream parity** — the stock core and the Rust core run the same
   ROM headlessly; video + audio + save-RAM are hashed and must match exactly,
   across repeated runs and an 8-game matrix.
3. **RetroAchievements-RAM proof** — the emulated main RAM (what RA actually
   reads) is snapshot-compared with a determinism mask; the Rust core must match
   the stock core on every deterministic byte.
4. **Real-game install** — runs as a normal libretro core in RetroArch.

All of this is driven by `melonbench`, a dependency-free Rust harness in this repo
(`melonbench suite --verify`, `melonbench ramverify`, `melonbench symbolize`, plus
a built-in sampling profiler).

## Repo layout

```
melonink/        Rust staticlib: the bit-exact ports (no_std, C ABI). The crate.
melonbench/      Rust harness: benchmarks, profiler, suite runner, ramverify.
patches/         melonds-rustymelon.patch — the additive C++ changes vs upstream.
benchmarks/      suite.cfg.example + (gitignored) results.
docs/            Architecture, verification methodology, accuracy results, roadmap.
custom-core/     (gitignored binaries) staging for the built practice core.
build.ps1        One-shot: fetch sources, apply patch, build melonink + the core.
```

The upstream `melonDS` and `melonds-ds` repos are **not** vendored here — they are
fetched at build time and the patch is applied on top (see `build.ps1`).

## Building

Requirements: a MinGW-w64 toolchain (gcc, CMake ≥ 3.19, Ninja) and Rust (stable,
`x86_64-pc-windows-gnu`). On Windows, MSYS2 provides the C++ toolchain.

```powershell
# fetches melonds-ds + melonDS@f6692df, applies patches/melonds-rustymelon.patch,
# builds the melonink staticlib, then builds the core with -DMELONINK_LIB=...
pwsh ./build.ps1
```

To build the **unmodified baseline** for comparison, configure without
`-DMELONINK_LIB` / `-DMELONINK` — the tree is then byte-for-byte upstream.

## Verifying

```powershell
# copy and edit paths first
cp benchmarks/suite.cfg.example benchmarks/suite.cfg

# one-command production gate: unit/fuzz tests + stream parity + RA-RAM proof.
# Exits non-zero on any failure (use as a CI / pre-release gate).
pwsh ./verify-all.ps1

# or the individual pieces:
# must print IDENTICAL for every game (exit 0)
melonbench suite --verify --frames 900 --cores original,rusty

# RA-visible RAM proof: dump 3 stock runs + 1 rusty run, then mask-compare
melonbench ramverify --ref s1.bin --ref s2.bin --ref s3.bin --cand rusty.bin
```

## RetroAchievements

rustymelon is built so it *could* eventually be accepted — but the honest path is
**upstreaming**, not shipping a private fork:

- The Rust changes are additive and `#ifdef`-guarded specifically so they can be
  proposed to official melonDS / melonds-ds. If merged, the *already-approved*
  core carries the speedups and nothing else changes.
- A standalone custom core is not on RA's allowed list, and its identity must
  **never** be spoofed to pass Hardcore's core check. Until approved, use the
  stock core for anything that counts.
- What we *can* prove — and do — is that emulated memory is byte-identical, so
  achievement logic behaves exactly as on the stock core. See
  [docs/RETROACHIEVEMENTS.md](docs/RETROACHIEVEMENTS.md).

## License

GPL-3.0-or-later, matching melonDS. The Rust `melonink` code links into the GPL
binary and is licensed under the same terms. See [LICENSE](LICENSE). melonDS is
© the melonDS team; the melonds-ds libretro core is © Jesse Talavera-Greenberg and
contributors; rustymelon's modifications are © their respective authors.
