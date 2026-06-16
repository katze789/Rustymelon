# RetroAchievements: where rustymelon stands, and the path to approval

This is an honest assessment, not a promise. rustymelon is engineered so it
*could* be accepted by RetroAchievements — but the realistic route is
**upstreaming the changes into official melonDS**, not getting a private fork
blessed.

## What RetroAchievements actually requires

RA Hardcore only trusts cores on its **allowed list**, and it enforces emulator
integrity. For a Nintendo DS core that means, in practice:

1. **Accurate, stable emulation** — achievement logic reads emulated console
   memory; the core must expose it correctly and behave deterministically.
2. **An approved build identity** — RA recognises specific cores. A custom binary
   is not recognised, and its identity must never be spoofed to pass the check.
3. **No Hardcore-illegal features active** — save states, rewind, slowdown, cheats
   are blocked in Hardcore (enforced by RetroArch/rcheevos, not the core).

melonDS DS is already an approved core. rustymelon changes only the *renderer*,
never the CPU, memory map, timing, or save behaviour.

## What we can prove (and do)

- **Emulated main RAM is byte-identical** to the stock core on every
  deterministic byte (`melonbench ramverify`), across an 8-game matrix — so every
  achievement evaluates exactly as it would on the approved core.
- **Video, audio, and save-RAM are bit-identical** over headless runs
  (`melonbench suite --verify`).
- The changes are **additive and `#ifdef MELONINK`-guarded**, so the same source
  builds byte-for-byte upstream with the flag off — i.e. trivially reviewable and
  mergeable.

See [VERIFICATION.md](VERIFICATION.md) and [ACCURACY.md](ACCURACY.md) for the
evidence.

## The recommended path: upstream, don't fork

1. **Propose the changes to melonDS / melonds-ds.** The patch
   (`patches/melonds-rustymelon.patch`) is small and additive. If the maintainers
   want the Rust optimisation, it ships in the official build and the approved
   core simply gets faster — no new RA approval needed.
2. If upstream prefers C++ over a Rust dependency, the same work can be presented
   as a reference for an equivalent C++ optimisation; the accuracy harness still
   proves the optimisation is behaviour-preserving.
3. **Only if** a standalone rustymelon core is genuinely wanted would you approach
   RA directly — and that means a full emulator-accuracy review and being added to
   the allowed list, which is a high bar for a fork of an existing core.

## What to do *before* any of that (readiness checklist)

- [x] Behaviour-preservation proven (stream + RA-RAM parity, fuzz tests).
- [x] Changes additive, guarded, and reviewable as a patch.
- [x] Honest core identity (installs as a clearly-labelled separate core).
- [x] GPL-3.0 compliance (full source + license + stated changes).
- [ ] **Wider game coverage** — more titles, longer and input-driven runs that
      reach actual gameplay, not just intros (see ROADMAP.md). RA cares about
      correctness deep into games.
- [ ] **A clean upstream PR** — rebased on current melonDS, with the accuracy
      evidence attached, and a decision from maintainers on Rust-vs-C++.
- [ ] **Sign-off from RA / speedrun moderators** before any non-practice use.

Until those last items are done, treat rustymelon strictly as a **local practice
build** and keep the stock core as the default for anything that counts.
