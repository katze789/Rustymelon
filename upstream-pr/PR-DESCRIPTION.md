# GPU2D: fast-path the color-effects pass when BLDCNT is 0

## Summary

`SoftRenderer2D::DrawScanline_BGOBJ` always runs the final color-special-effects
pass as a 256-iteration loop that calls `ColorComposite()` once per pixel. When
no blend/brightness effect is enabled (`BLDCNT == 0`), `ColorComposite()` returns
its source pixel unchanged, so that loop just copies `BGOBJLine` to `dst`. This PR
detects that case and replaces the per-pixel calls with a single `memcpy`.

The existing code already carries the comment `// can likely be optimized` on this
loop; this is that optimization, in its safest form.

## Why it's correct (behavior-preserving)

`ColorComposite(i, val1, val2)` only returns something other than `val1` when
`coloreffect != 0`, and every branch that sets `coloreffect` is gated on a bit of
`blendCnt` (`GPU2D.BlendCnt`):

- sprite blending requires `blendCnt & target2`,
- 3D-layer blending requires `blendCnt & target2`,
- the DISPCNT brightness/blend effect requires `blendCnt & flag1`.

So when `blendCnt == 0`, all branches fall through, `coloreffect` stays `0`, and
the function returns `val1` for **every** pixel. Since `val1 == BGOBJLine[i]`, the
loop is exactly `dst[i] = BGOBJLine[i]` — i.e. `memcpy(dst, BGOBJLine, 256*4)`.
The output is bit-identical; only the no-effect case is made faster.

## Performance

It eliminates 256 indirect-free but non-trivial function calls (plus the branchy
`ColorComposite` body) on every scanline where no color effect is active — which
is the common case for many frames/games — replacing them with one `memcpy` of
1 KiB. It is strictly never slower: the `BLDCNT != 0` path is byte-for-byte the
original loop. (I haven't isolated a standalone FPS number on master; happy to
provide benchmarks if useful.)

## Testing

The behavior-preservation was validated with a headless harness that runs the
stock build and the modified build on the same ROMs and compares **per-frame
hashes of video + audio + save-RAM**, plus a masked main-RAM snapshot diff (the
memory RetroAchievements reads), across an 8-game matrix including input-driven
gameplay. The equivalent change (skip the no-op composite) was part of that work
and stayed bit-identical on every game. The correctness here also follows directly
from the argument above, which holds identically on current master (verified by
reading `ColorComposite` on master).

I'll run melonDS's own CI/test suite on the PR branch as well.

## Notes

- Plain C++, no new dependencies, C++17, follows the melonDS style guide.
- `memcpy`/`<string.h>` is already used in this file.
- Scope: only the software renderer's 2D `DrawScanline_BGOBJ`. The OpenGL renderer
  is untouched.
