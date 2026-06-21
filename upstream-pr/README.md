# upstream-pr — a C++ optimization ready to propose to melonDS

This folder contains a **plain-C++**, dependency-free optimization extracted from
the rustymelon work, packaged so it can be proposed upstream — to
**[melonDS-emu/melonDS]** (the emulator), *not* the libretro wrapper. (melonds-ds's
own CONTRIBUTING says renderer improvements should go upstream first, and both
projects forbid new dependencies — so nothing here uses Rust.)

## Contents

| File | What |
|---|---|
| `0001-gpu2d-fast-path-color-effects-when-blendcnt-zero.patch` | the change, as a `git apply`-able diff against **melonDS master** (`10a173b` at time of writing) |
| `PR-DESCRIPTION.md` | a ready-to-paste pull-request description |
| `NOTES-rust-port-vs-portable-cpp.md` | honest notes: what from the Rust port is a *real portable optimization* vs. what was 1:1 translation / codegen / measurement noise |

## The optimization in one sentence

In `SoftRenderer2D::DrawScanline_BGOBJ`, when `BLDCNT == 0` no color special
effect can apply, so `ColorComposite()` returns the source pixel unchanged for
every pixel — the whole pass is a plain copy, and the 256 per-pixel function
calls can be replaced with a single `memcpy`. (Upstream already flags this loop
with `// can likely be optimized`.)

## Applying / testing it

```sh
git clone https://github.com/melonDS-emu/melonDS
cd melonDS
git apply ../upstream-pr/0001-gpu2d-fast-path-color-effects-when-blendcnt-zero.patch
# build melonDS as usual and run its own test suite
```

Before opening a PR, follow melonDS's process: it's small, but courtesy +
the CONTRIBUTING note both suggest floating the idea with the maintainers first.

[melonDS-emu/melonDS]: https://github.com/melonDS-emu/melonDS
