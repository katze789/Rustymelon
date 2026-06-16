# Verification methodology

This fork's central claim is **behavior preservation**: every Rust component
produces output bit-identical to the upstream melonDS C++ it replaces, so the
emulator stays accurate and RetroAchievements-correct. This document describes
how that claim is tested. Nothing here is published; all artifacts stay local.

There are four independent layers of evidence, from unit-level to whole-system.

## 1. In-crate equivalence tests (`melonink`)

Each ported function has a Rust unit-test suite that compares it against an
**independently transcribed reference** of the same C++ logic. Two separate
transcriptions of the same algorithm, fuzzed against each other over millions
of inputs, catch transcription mistakes that a single transcription cannot.

- **ColorComposite** (`src/composite.rs`): exhaustive over all 65 536 flag-byte
  pairs × 6 register configs × both window-mask states; plus 2 000 random full
  scanlines comparing the AVX2 path against the scalar path.
- **TextureLookup** (`src/texture.rs`): 3 000 000 random cases over random VRAM,
  sweeping all 8 texture formats, all width/height sizes, all wrap/flip modes,
  the colour-0-transparent flag, random palettes and texcoords; plus focused
  golden-value tests per format.

Run: `cargo test --release` in `melonink/`.

These prove the **scalar and SIMD Rust agree with a second human reading of the
C++**. They do not, by themselves, prove agreement with the *actual* compiled
C++ — that is layers 3 and 4.

## 2. Isolation microbenchmarks (`melonink/examples/bench.rs`)

`cargo run --release --example bench` reports ns/scanline for scalar vs SIMD on
representative line mixes. Used to decide whether a port is worth shipping
*before* touching the core. Not a correctness check.

## 3. Whole-core stream parity (`melonbench suite --verify`)

The real differential test against the *compiled upstream C++*. The harness
loads the **stock core** and the **Rust core** headlessly, runs the same ROM
with the same inputs under deterministic settings (fixed RTC, silent mic), and
compares rolling FNV-1a hashes of:

- the **video** stream (every framebuffer, every frame),
- the **audio** stream (every sample), and
- **save-RAM**.

A game passes only if all three are identical across both cores and repeated
runs. Because the stock core is the reference emulator, identical hashes mean
the Rust path reproduced its output exactly on the content the ROM exercised.

Run: `melonbench suite --verify --frames 500 --cores original,rusty`

### Determinism caveat
A test ROM is only a valid strict-hash oracle where the **stock core is
deterministic run-to-run**. Confirmed:
- **MKDS, SM64DS, NSMB, Pokémon**: deterministic to 900+ frames.
- **CTGP Nitro**: deterministic only to ~600 frames — beyond that its WiFi/
  online code reacts to host timing and the *stock core disagrees with itself*.
  Use `--frames 500` for CTGP. (This was verified by running the stock core
  against itself; see `docs/ACCURACY.md`.)

## 4. RetroAchievements-RAM proof (`melonbench ramverify`)

RetroAchievements does not read the screen — it reads the emulated console's
**main RAM**. So the achievement-correctness question is precisely: *does the
Rust core leave main RAM byte-identical to the stock core?*

A naive full-RAM hash is too strict: the stock core itself varies by ~3 bytes
out of 4 MB run-to-run (uninitialised scratch the game never reads). So:

1. Dump main RAM from **two stock-core runs** (`--ram-dump`).
2. Build a **determinism mask** = bytes where the two stock runs agree.
3. Require the **candidate core** to match the stock core on *every* byte in
   that mask.

A pass means nothing RetroAchievements can observe has changed.

```
melonbench --core <stock> ... --ram-dump stock1.bin
melonbench --core <stock> ... --ram-dump stock2.bin
melonbench --core <rusty> ... --ram-dump rusty.bin
melonbench ramverify --ref-a stock1.bin --ref-b stock2.bin --cand rusty.bin
```

## What "ready for RA" requires beyond this

These layers establish *behavioural* equivalence. RetroAchievements **Hardcore**
additionally requires the core to be an *approved build* — a custom binary is
not on the allowed list regardless of how accurate it is, and the core identity
must **never** be spoofed to bypass that check. The legitimate path is to get
the changes merged into upstream melonDS so the official approved build carries
them. The C++ integration is kept upstream-shaped for that reason. Until then
the Rust core is a **local practice build** and the stock core remains the
default for anything that counts.
