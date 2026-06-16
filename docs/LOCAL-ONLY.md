# Distribution & publishing — obligations and pre-flight

The repo is **prepared for GitHub** (README, LICENSE, `.gitignore`, build script,
patch, docs). Whether to actually publish/push is a deliberate decision for the
project owner. This document lists what must be true *before* any public release.

## Hard requirements before publishing

1. **GPL-3.0 compliance.** melonDS and the melonds-ds core are GPL-3.0. Any
   public distribution — source or binary — must:
   - include the full license ([../LICENSE](../LICENSE)), preserve all upstream
     copyright notices,
   - provide the **complete corresponding source** (the Rust `melonink` crate is
     GPL-compatible and licensed the same; the C++ changes ship as
     `patches/melonds-rustymelon.patch`),
   - **state the changes** made (the README + patch do this).
   - Do **not** commit prebuilt `.dll`/`.a` binaries — ship source + `build.ps1`.
     (The `.gitignore` already excludes binaries and the stock cores.)

2. **No redistribution of the stock cores or BIOS/firmware/ROMs.** Those are not
   ours. `original-cores-backup/` and any BIOS/ROM paths stay out of the repo.

3. **RetroAchievements integrity.** The build must keep identifying itself
   honestly as a custom core and must **never** spoof the official core's identity
   to pass RA Hardcore's allowed-core check. See [RETROACHIEVEMENTS.md](RETROACHIEVEMENTS.md).

## Recommended path

The strongest outcome is **upstreaming** the change into official melonDS rather
than maintaining a separate published fork (see RETROACHIEVEMENTS.md). If a public
rustymelon repo is still wanted, it should be framed clearly as an
experimental/practice optimisation with the accuracy evidence attached.

## What "ready to push" means here

- `git init` + an initial commit can be made locally at any time; the `.gitignore`
  keeps clones, binaries, results, and machine-specific config out.
- **Pushing to a remote is a separate, explicit step** — it has not been done and
  should only happen once the requirements above are satisfied and the owner
  decides to publish.
