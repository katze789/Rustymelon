# rustymelon docs

A local, accuracy-first Rust fork of the melonDS DS libretro core. Goal: move
hot/bug-prone paths to Rust while staying **bit-identical** to upstream, so the
emulator remains accurate and RetroAchievements-correct — and could *eventually*
be approved by RA via upstream merge.

Read in this order:

1. **[ARCHITECTURE.md](ARCHITECTURE.md)** — what the pieces are, how Rust is
   linked into the C++ core, what's ported so far.
2. **[VERIFICATION.md](VERIFICATION.md)** — the four-layer test methodology
   (unit/fuzz, microbench, whole-core stream parity, RA-RAM proof).
3. **[ACCURACY.md](ACCURACY.md)** — current results: every test, every game.
4. **[ROADMAP.md](ROADMAP.md)** — what's done and what's next.
5. **[RETROACHIEVEMENTS.md](RETROACHIEVEMENTS.md)** — where rustymelon stands for
   RA approval, and the path (upstreaming).
6. **[LOCAL-ONLY.md](LOCAL-ONLY.md)** — publishing/distribution obligations
   (GPL-3.0, RA integrity) and what must be true before any release.

Quick commands (run from the project root):

```powershell
# unit + fuzz tests
cd melonink; cargo test --release

# prove the core is behaviour-identical to stock, then measure
melonbench suite --verify --frames 900 --cores original,rusty
melonbench suite --frames 2000 --warmup 300 --reps 3 --cores baseline,rusty
```
