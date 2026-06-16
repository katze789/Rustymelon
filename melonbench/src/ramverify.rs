//! `melonbench ramverify` — prove a candidate core leaves the emulated main
//! RAM (the memory RetroAchievements reads) byte-identical to the stock core
//! on every *deterministic* byte.
//!
//! Even the stock core has a few bytes of main RAM that vary run-to-run
//! (uninitialised scratch the game never reads — confirmed ~3 bytes / 4 MB).
//! Hashing all of RAM would therefore flag the stock core against itself. So
//! we first derive a determinism mask from two stock-core runs (bytes that
//! agree with themselves), then require the candidate to match the stock core
//! on exactly those bytes. A clean pass means: nothing RetroAchievements can
//! read has changed.

use std::path::Path;

fn read(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
}

fn ram_usage() -> ! {
    eprintln!(
        "usage: melonbench ramverify --ref A.bin --ref B.bin [--ref C.bin ...] --cand X.bin [--cand ...]

  --ref FILE    SYSTEM_RAM snapshot from the STOCK core (same ROM, same
                inputs). Need >= 2. A byte is 'deterministic' only if it is
                equal across ALL refs — more refs = stricter, leak-proof mask
                (important for ROMs whose RAM is heavily nondeterministic, e.g.
                Pokemon's RNG/RTC state).
  --cand FILE   candidate-core snapshot to check (repeatable).

  (legacy --ref-a/--ref-b accepted as aliases for two --ref.)

Exit 0 only if every candidate matches the refs on all deterministic bytes."
    );
    std::process::exit(2);
}

pub fn run_ramverify(argv: &[String]) {
    let mut refs: Vec<String> = Vec::new();
    let mut cands: Vec<String> = Vec::new();
    let mut i = 0;
    let next = |i: &mut usize| -> String {
        *i += 1;
        argv.get(*i).cloned().unwrap_or_else(|| ram_usage())
    };
    while i < argv.len() {
        match argv[i].as_str() {
            "--ref" | "--ref-a" | "--ref-b" => refs.push(next(&mut i)),
            "--cand" => cands.push(next(&mut i)),
            _ => ram_usage(),
        }
        i += 1;
    }
    if refs.len() < 2 || cands.is_empty() {
        ram_usage();
    }

    let ref_data: Vec<Vec<u8>> = refs.iter().map(|p| read(p)).collect();
    let len = ref_data[0].len();
    for (p, d) in refs.iter().zip(&ref_data) {
        if d.len() != len {
            eprintln!("reference {p} differs in size ({} vs {len})", d.len());
            std::process::exit(2);
        }
    }
    let a = &ref_data[0];

    // Determinism mask: true where ALL stock runs agree byte-for-byte.
    let mut deterministic = vec![true; len];
    let mut nondet = 0usize;
    for i in 0..len {
        let v = a[i];
        if ref_data.iter().any(|r| r[i] != v) {
            deterministic[i] = false;
            nondet += 1;
        }
    }
    let det = len - nondet;
    println!(
        "reference: {} bytes, {} deterministic, {} nondeterministic ({:.6}%) across {} stock runs",
        len,
        det,
        nondet,
        100.0 * nondet as f64 / len as f64,
        ref_data.len()
    );

    let mut all_pass = true;
    for cand_path in &cands {
        let c = read(cand_path);
        let name = Path::new(cand_path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| cand_path.clone());
        if c.len() != a.len() {
            println!("{name}: SIZE MISMATCH ({} vs {})", c.len(), a.len());
            all_pass = false;
            continue;
        }
        let mut mismatches = 0usize;
        let mut first = Vec::new();
        for i in 0..a.len() {
            if deterministic[i] && c[i] != a[i] {
                mismatches += 1;
                if first.len() < 8 {
                    first.push((i, a[i], c[i]));
                }
            }
        }
        if mismatches == 0 {
            println!("{name}: PASS — identical on all {det} deterministic bytes");
        } else {
            all_pass = false;
            println!("{name}: FAIL — {mismatches} deterministic bytes differ (RA-visible!)");
            for (off, want, got) in first {
                println!("    0x{off:06x}: stock={want:02x} cand={got:02x}");
            }
        }
    }

    if !all_pass {
        std::process::exit(1);
    }
    println!("\nALL CANDIDATES RA-SAFE: emulated RAM unchanged where it is observable.");
}
