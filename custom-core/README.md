# melonDS DS "rusty" — custom practice build

**This is a custom/dev build. Practice-only.** Do not use it for
RetroAchievements Hardcore sessions or submitted speedruns unless RA /
the relevant moderators have explicitly approved custom builds. Your
original core at `C:\RetroArch-Win64\cores\melondsds_libretro.dll` is
untouched — keep using it for anything that counts.

## What's different

Built from the same melonds-ds 1.2.0 source as the official core, plus:
the 2D color-effect compositor scanline loop is implemented in Rust
(`melonink` crate) with an AVX2 SIMD path and adaptive early-outs.

**Verified behavior-identical** to the original core: bit-equal video and
audio streams over deterministic runs of CTGP Nitro, Mario Kart DS, and
SM64DS The New Beginning (see `benchmarks\results\verify-*`). In isolation
the new compositor is 1.6–15x faster than the C++ loop; whole-game effect
on this laptop is small (the compositor is ~5% of frame time).

## To try it

Copy `melondsds_rusty_libretro.dll` into `C:\RetroArch-Win64\cores\` and
`melondsds_rusty_libretro.info` into `C:\RetroArch-Win64\info\`, then pick
"Nintendo - DS (melonDS DS RUSTY custom practice build)" when loading a
game. It appears as a separate core alongside the originals; it shares the
same core options. To remove it, delete those two files.
