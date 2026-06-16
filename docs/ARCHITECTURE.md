# Architecture

A local, accuracy-first Rust fork of the **melonDS DS** libretro core. The
strategy is *not* a rewrite: the upstream C++ emulator stays intact, and
self-contained hot/bug-prone routines are replaced one at a time with Rust that
is proven bit-identical (see `VERIFICATION.md`).

## Components

```
rustymelon ds ds/
├── melonds-ds/            JesseTG/melonds-ds (libretro core) — clone, reference
├── melonDS/               melonDS-emu/melonDS — clone, reference only
├── melonink/              Rust staticlib: bit-exact ports of hot paths (no_std)
├── melonbench/            Rust harness: benchmarks, profiler, verification tools
├── benchmarks/            suite.cfg + timestamped results
├── original-cores-backup/ untouched stock core DLLs (+ SHA256)
├── custom-core/           the built Rust core, staged for local practice use
└── docs/                  this documentation
```

Heavy/dirty build artifacts live on `E:\rustymelon-work\` (toolchain, CMake
build trees, the patched melonDS source, the benchmark sandbox) because the C:
drive is space-constrained.

## How Rust gets into the core

The melonDS C++ that melonds-ds compiles is fetched by CMake. We point that at a
**local patched checkout** (`E:\rustymelon-work\melonDS-patched`, pinned to the
same commit upstream uses) via `-DFETCHCONTENT_SOURCE_DIR_MELONDS=...`. In that
checkout, each ported routine is guarded:

```cpp
#ifdef MELONINK
    // call into the Rust staticlib over a C ABI
    melonink_xxx(...);
    return;          // (or replace just the inner loop)
#else
    ... original upstream C++ ...
#endif
```

`melonink` is built as a `staticlib` (`cargo build --release
--no-default-features`, producing `libmelonink.a`) and linked in when CMake is
configured with `-DMELONINK_LIB=<path to libmelonink.a>`. With the flag unset,
the build is byte-for-byte upstream — which is how the "baseline" core in the
benchmarks is produced, and what keeps the change reviewable/upstreamable.

### Why `no_std` + `panic = "abort"`
The staticlib links into a C++ binary that has no Rust runtime. `no_std` keeps
it free of the Rust std runtime; `panic = "abort"` avoids unwinding across the
language boundary. The ported routines are *total* (no panics on any input); a
no-op `rust_eh_personality` stub satisfies the linker for the unwinding symbols
the prebuilt `core` crate still references but never reaches.

### Data crosses the boundary by pointer, not by callback
Ported routines get direct pointers to the C++ data they need and index it
exactly as the C++ does — e.g. TextureLookup receives `gpu.VRAMFlat_Texture`
(512 KiB) and `gpu.VRAMFlat_TexPal` (128 KiB) and reads them itself, rather than
calling back into C++ per texel. This is what makes the boundary cheap enough to
be a net win, and it is granular: the compositor processes a whole 256-pixel
scanline per call.

## Ported so far (`melonink`)

| Routine | Upstream location | Notes |
|---|---|---|
| `GPU2D::SoftRenderer::ColorComposite` scanline loop | `GPU2D_Soft.cpp` | AVX2 + scalar, adaptive early-outs |
| `GPU3D::SoftRenderer::TextureLookup` | `GPU3D_Soft.cpp` | all 8 NDS texture formats |
| `GPU3D::SoftRenderer::RenderPixel` | `GPU3D_Soft.cpp` | pure per-pixel shader (toon/decal/modulate) |
| `RenderPolygonScanline` per-pixel span loop | `GPU3D_Soft.cpp` | the three span loops + `Interpolator<0>`, the 4 depth tests, `AlphaBlend`, `PlotTranslucentPixel`. Called once per scanline (`melonink_render_spans`), so texture/render-pixel run with no per-pixel FFI hop. |
| `GPU2D::SoftRenderer::DrawBG_Text` pixel loops | `GPU2D_Soft.cpp` | 256/16-colour text-BG drawing (mosaic + ext-palette + both DrawPixel variants). VRAM-bank setup (`GetBGVRAM`/`GetBGExtPal`) stays in C++; the 16 ext-palette pointers are precomputed and passed in. |
| `GPU2D::SoftRenderer::DrawSprite_Normal` + `DrawSprite_Rotscale` | `GPU2D_Soft.cpp` | full sprite rendering: bitmap / 256-colour / 16-colour, normal + affine (rotscale), window sprites, both flips, mosaic. Writes raw indices to `OBJLine`/`OBJWindow` (palette lookup stays in C++ `InterleaveSprites`); VRAM coherency + OAM iteration stay in C++. |

Each is exercised behind `#ifdef MELONINK` and verified per `VERIFICATION.md`.
The per-scanline setup (slope/edge setup, Y-interpolation of span endpoints)
stays in C++; only the hot per-pixel work crossed into Rust.

## Build / verify / benchmark flow

```powershell
# 1. build the Rust staticlib
cargo build --release --no-default-features   # in melonink/  (target on E:)
# 2. (re)build the core with melonink linked
cmake --build E:/rustymelon-work/build/melondsds-rust
# 3. prove behaviour unchanged
melonbench suite --verify --frames 500 --cores original,rusty   # video+audio+saveRAM
melonbench ramverify --ref-a s1.bin --ref-b s2.bin --cand r.bin # RA-visible RAM
# 4. measure
melonbench suite --frames 2000 --warmup 300 --reps 3 --cores baseline,rusty
```

Cores referenced by the suite (see `benchmarks/suite.cfg`):
- **original** — the stock buildbot DLL (reference).
- **baseline** — our build from unmodified source, `MELONINK` off (proves the
  build itself is faithful before any Rust is added).
- **rusty** — the build with `melonink` linked (compositor + texture).

## The toolchain-vanished incident
`C:\msys64` (MinGW/CMake/Ninja) was removed mid-development by a disk cleanup or
AV sweep when C: was low on space. Recovery: run
`E:\rustymelon-work\msys2-installer.exe in --confirm-command --accept-messages
--root C:/msys64` (winget refuses because the stale registry entry persists),
then `pacman -S --needed mingw-w64-x86_64-gcc mingw-w64-x86_64-cmake
mingw-w64-x86_64-ninja git make`. Consider excluding `C:\msys64` from cleanup/AV.
