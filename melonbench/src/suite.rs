//! `melonbench suite` — run cores × games × reps, aggregate, and (in verify
//! mode) enforce hash equivalence. Rust port of the former bench-suite.ps1.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[repr(C)]
struct SYSTEMTIME {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    ms: u16,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLocalTime(t: *mut SYSTEMTIME);
}

fn timestamp() -> String {
    unsafe {
        let mut t: SYSTEMTIME = core::mem::zeroed();
        GetLocalTime(&mut t);
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            t.year, t.month, t.day, t.hour, t.minute, t.second
        )
    }
}

#[derive(Default)]
struct Config {
    cores: Vec<(String, String)>,
    games: Vec<Game>,
    system_dir: String,
    save_dir: String,
    opt_file: String,
    results_dir: String,
}

/// A test ROM plus any quirks that limit how it may be hash-compared. Some
/// stock-core ROMs are not fully deterministic run-to-run (e.g. WiFi/online
/// timing, or threaded-SPU audio jitter); flag those so the equivalence gate
/// only compares the streams that ARE deterministic for them.
#[derive(Clone)]
struct Game {
    label: String,
    path: String,
    audio_nondet: bool,
    video_nondet: bool,
    // Documentary: main RAM is too nondeterministic run-to-run for byte-level
    // ramverify to be meaningful (see docs/ACCURACY.md). Does not affect the
    // stream gate; recorded so the flag is recognised and self-documenting.
    #[allow(dead_code)]
    ram_nondet: bool,
    max_frames: Option<u64>,
}

fn parse_game(rest: &str) -> Option<Game> {
    let (label, after) = rest.split_once('=')?;
    // optional "; flag1, flag2=val" suffix (paths never contain ';')
    let (path, flagstr) = match after.split_once(';') {
        Some((p, f)) => (p, f),
        None => (after, ""),
    };
    let mut g = Game {
        label: label.trim().to_string(),
        path: path.trim().to_string(),
        audio_nondet: false,
        video_nondet: false,
        ram_nondet: false,
        max_frames: None,
    };
    for flag in flagstr.split(',') {
        let flag = flag.trim();
        if flag.is_empty() {
            continue;
        }
        match flag.split_once('=') {
            Some(("frames", n)) => g.max_frames = n.trim().parse().ok(),
            _ => match flag {
                "audio_nondet" => g.audio_nondet = true,
                "video_nondet" => g.video_nondet = true,
                "ram_nondet" => g.ram_nondet = true,
                other => eprintln!("suite config: unknown game flag '{other}'"),
            },
        }
    }
    Some(g)
}

fn load_config(path: &Path) -> Config {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read suite config {}: {e}", path.display()));
    let mut cfg = Config::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, rest) = match line.split_once(' ') {
            Some(kv) => kv,
            None => continue,
        };
        let rest = rest.trim();
        match key {
            "core" => {
                if let Some((label, path)) = rest.split_once('=') {
                    cfg.cores.push((label.trim().to_string(), path.trim().to_string()));
                }
            }
            "game" => {
                if let Some(g) = parse_game(rest) {
                    cfg.games.push(g);
                }
            }
            "system_dir" => cfg.system_dir = rest.to_string(),
            "save_dir" => cfg.save_dir = rest.to_string(),
            "opt_file" => cfg.opt_file = rest.to_string(),
            "results_dir" => cfg.results_dir = rest.to_string(),
            _ => eprintln!("suite config: unknown key '{key}'"),
        }
    }
    cfg
}

// Minimal extractors for melonbench's own JSON output (known fixed shape).
fn json_f64(json: &str, key: &str) -> f64 {
    json_raw(json, key)
        .and_then(|v| v.trim_end_matches(['}', ',']).trim().parse().ok())
        .unwrap_or(0.0)
}

fn json_u64(json: &str, key: &str) -> u64 {
    json_f64(json, key) as u64
}

fn json_str(json: &str, key: &str) -> String {
    json_raw(json, key)
        .map(|v| v.trim().trim_matches(['"', ',']).to_string())
        .unwrap_or_default()
}

fn json_raw<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\":");
    let start = json.find(&pat)? + pat.len();
    let rest = &json[start..];
    let end = rest.find(['\n', ','])?;
    Some(&rest[..end])
}

struct RunResult {
    core: String,
    game: String,
    rep: u32,
    avg_fps: f64,
    speed_x: f64,
    p99: f64,
    max: f64,
    over_budget: u64,
    video_hash: String,
    audio_hash: String,
    ram_hash: String,
    saveram_hash: String,
}

struct SuiteArgs {
    config: Option<String>,
    frames: u64,
    warmup: u64,
    reps: u32,
    verify: bool,
    core_filter: Vec<String>,
    game_filter: Vec<String>,
}

fn suite_usage() -> ! {
    eprintln!(
        "usage: melonbench suite [options]

options:
  --config FILE   suite config (default: suite.cfg or benchmarks\\suite.cfg)
  --frames N      measured frames per run (default 3000)
  --warmup N      warmup frames (default 300; verify mode forces 0)
  --reps N        repetitions (default 3; verify mode uses 2)
  --verify        deterministic mode + hash-equivalence gate
  --cores a,b     only these core labels
  --games x,y     only these game labels"
    );
    std::process::exit(2);
}

fn parse_suite_args(argv: &[String]) -> SuiteArgs {
    let mut a = SuiteArgs {
        config: None,
        frames: 3000,
        warmup: 300,
        reps: 3,
        verify: false,
        core_filter: Vec::new(),
        game_filter: Vec::new(),
    };
    let mut i = 0;
    let next = |i: &mut usize| -> String {
        *i += 1;
        argv.get(*i).cloned().unwrap_or_else(|| suite_usage())
    };
    while i < argv.len() {
        match argv[i].as_str() {
            "--config" => a.config = Some(next(&mut i)),
            "--frames" => a.frames = next(&mut i).parse().unwrap_or_else(|_| suite_usage()),
            "--warmup" => a.warmup = next(&mut i).parse().unwrap_or_else(|_| suite_usage()),
            "--reps" => a.reps = next(&mut i).parse().unwrap_or_else(|_| suite_usage()),
            "--verify" => a.verify = true,
            "--cores" => a.core_filter = next(&mut i).split(',').map(|s| s.trim().to_string()).collect(),
            "--games" => a.game_filter = next(&mut i).split(',').map(|s| s.trim().to_string()).collect(),
            _ => suite_usage(),
        }
        i += 1;
    }
    a
}

fn find_config(explicit: &Option<String>) -> PathBuf {
    if let Some(p) = explicit {
        return PathBuf::from(p);
    }
    for cand in ["suite.cfg", "benchmarks\\suite.cfg"] {
        let p = PathBuf::from(cand);
        if p.exists() {
            return p;
        }
    }
    eprintln!("no suite.cfg found (looked in . and .\\benchmarks); pass --config");
    std::process::exit(2);
}

pub fn run_suite(argv: &[String]) {
    let args = parse_suite_args(argv);
    let cfg = load_config(&find_config(&args.config));

    let keep = |filter: &[String], label: &str| filter.is_empty() || filter.iter().any(|f| f == label);
    let cores: Vec<_> = cfg
        .cores
        .iter()
        .filter(|(l, p)| {
            if !keep(&args.core_filter, l) {
                return false;
            }
            if Path::new(p).exists() {
                true
            } else {
                eprintln!("skipping core '{l}' (not found: {p})");
                false
            }
        })
        .collect();
    let games: Vec<_> = cfg
        .games
        .iter()
        .filter(|g| {
            if !keep(&args.game_filter, &g.label) {
                return false;
            }
            if Path::new(&g.path).exists() {
                true
            } else {
                eprintln!("skipping game '{}' (not found: {})", g.label, g.path);
                false
            }
        })
        .collect();
    if cores.is_empty() || games.is_empty() {
        eprintln!("nothing to run (no cores or no games)");
        std::process::exit(2);
    }

    let stamp = timestamp();
    let suite_name = format!("{}-{stamp}", if args.verify { "verify" } else { "perf" });
    let out_dir = PathBuf::from(&cfg.results_dir).join(&suite_name);
    std::fs::create_dir_all(&out_dir).expect("create results dir");

    let exe = std::env::current_exe().expect("current_exe");
    let reps = if args.verify { 2 } else { args.reps };
    let mut results: Vec<RunResult> = Vec::new();

    for game in &games {
        let game_label = &game.label;
        let rom = &game.path;
        // Honour a per-game frame cap (e.g. CTGP is only deterministic <=500f).
        let frames = match game.max_frames {
            Some(cap) if args.verify => args.frames.min(cap),
            _ => args.frames,
        };
        if frames != args.frames {
            eprintln!("[{game_label}] frame cap {frames} (config: max_frames)");
        }
        for rep in 1..=reps {
            // Alternate order each rep: clocks sag monotonically during a
            // suite, so a fixed order biases against later cores.
            let mut order: Vec<_> = cores.clone();
            if rep % 2 == 0 {
                order.reverse();
            }
            for (core_label, core_path) in order {
                let label = format!("{core_label}--{game_label}--r{rep}");
                let json_path = out_dir.join(format!("{label}.json"));
                let mut cmd = Command::new(&exe);
                cmd.arg("--core").arg(core_path)
                    .arg("--rom").arg(rom)
                    .arg("--system-dir").arg(&cfg.system_dir)
                    .arg("--save-dir").arg(&cfg.save_dir)
                    .arg("--opt").arg(&cfg.opt_file)
                    .arg("--frames").arg(frames.to_string())
                    .arg("--warmup").arg(if args.verify { "0".into() } else { args.warmup.to_string() })
                    .arg("--json").arg(&json_path)
                    .arg("--label").arg(&label)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                if args.verify {
                    cmd.arg("--verify");
                }
                eprint!("[{label}] running...");
                let status = cmd.status().expect("spawn melonbench run");
                if !status.success() || !json_path.exists() {
                    eprintln!(" FAILED ({status})");
                    continue;
                }
                let json = std::fs::read_to_string(&json_path).expect("read run json");
                let r = RunResult {
                    core: core_label.clone(),
                    game: game_label.clone(),
                    rep,
                    avg_fps: json_f64(&json, "avg_fps"),
                    speed_x: json_f64(&json, "speed_x"),
                    p99: json_f64(&json, "p99"),
                    max: json_f64(&json, "max"),
                    over_budget: json_u64(&json, "frames_over_budget"),
                    video_hash: json_str(&json, "video_hash"),
                    audio_hash: json_str(&json, "audio_hash"),
                    ram_hash: json_str(&json, "ram_hash"),
                    saveram_hash: json_str(&json, "saveram_hash"),
                };
                eprintln!(" {:.1} fps ({:.2}x)", r.avg_fps, r.speed_x);
                results.push(r);
            }
        }
    }

    println!("\n=== Summary ({suite_name}) ===");
    println!(
        "{:<10} {:<15} {:>3} {:>8} {:>7} {:>8} {:>8} {:>10}",
        "Core", "Game", "Rep", "AvgFps", "SpeedX", "P99ms", "Maxms", "OverBudget"
    );
    for r in &results {
        println!(
            "{:<10} {:<15} {:>3} {:>8.1} {:>7.2} {:>8.2} {:>8.2} {:>10}",
            r.core, r.game, r.rep, r.avg_fps, r.speed_x, r.p99, r.max, r.over_budget
        );
    }

    println!("\n=== Median across reps ===");
    let mut groups: BTreeMap<(String, String), Vec<&RunResult>> = BTreeMap::new();
    for r in &results {
        groups.entry((r.core.clone(), r.game.clone())).or_default().push(r);
    }
    println!(
        "{:<10} {:<15} {:>10} {:>7} {:>8}",
        "Core", "Game", "MedianFps", "SpeedX", "P99ms"
    );
    for ((core, game), mut rs) in groups {
        rs.sort_by(|a, b| a.avg_fps.partial_cmp(&b.avg_fps).unwrap());
        let m = rs[(rs.len() - 1) / 2];
        println!(
            "{:<10} {:<15} {:>10.1} {:>7.2} {:>8.2}",
            core, game, m.avg_fps, m.speed_x, m.p99
        );
    }

    if args.verify {
        println!("\n=== Hash equivalence check ===");
        let flags: std::collections::HashMap<&str, &Game> =
            games.iter().map(|g| (g.label.as_str(), *g)).collect();
        let mut bad = false;
        let mut by_game: BTreeMap<&str, Vec<&RunResult>> = BTreeMap::new();
        for r in &results {
            by_game.entry(&r.game).or_default().push(r);
        }
        for (game, rs) in by_game {
            let uniq = |f: &dyn Fn(&RunResult) -> &str| -> Vec<String> {
                let mut v: Vec<String> = rs.iter().map(|r| f(r).to_string()).collect();
                v.sort_unstable();
                v.dedup();
                v
            };
            let vh = uniq(&|r| r.video_hash.as_str());
            let ah = uniq(&|r| r.audio_hash.as_str());
            let rh = uniq(&|r| r.ram_hash.as_str());
            let sh = uniq(&|r| r.saveram_hash.as_str());

            // Per-game flags: some stock ROMs are themselves nondeterministic on
            // a stream run-to-run (WiFi timing, threaded-SPU audio jitter), so
            // that stream can't be a strict oracle for them. Skip it in the gate
            // but still report and require the OTHER streams to match.
            let g = flags.get(game);
            let skip_audio = g.map(|g| g.audio_nondet).unwrap_or(false);
            let skip_video = g.map(|g| g.video_nondet).unwrap_or(false);

            // Full-RAM rolling hash is informational only: the stock core varies
            // by a few uninitialised scratch bytes run-to-run — use `ramverify`
            // (masked snapshot) for the authoritative RetroAchievements-RAM proof.
            let video_ok = skip_video || vh.len() == 1;
            let audio_ok = skip_audio || ah.len() == 1;
            let saveram_ok = sh.len() == 1;

            if video_ok && audio_ok && saveram_ok {
                let mut parts = Vec::new();
                if !skip_video { parts.push("video"); }
                if !skip_audio { parts.push("audio"); }
                parts.push("saveRAM");
                if rh.len() == 1 { parts.push("RAM"); }
                let mut note = String::new();
                if skip_video { note.push_str("  [video nondet in stock: skipped]"); }
                if skip_audio { note.push_str("  [audio nondet in stock: skipped]"); }
                if rh.len() != 1 { note.push_str("  (RAM scratch varies — confirm via ramverify)"); }
                println!("{game}: IDENTICAL {}{}", parts.join("+"), note);
            } else {
                bad = true;
                println!(
                    "{game}: MISMATCH! video={vh:?} audio={ah:?} saveram={sh:?}"
                );
            }
        }
        if bad {
            println!("BEHAVIOR DIFFERS — do not ship this build.");
            std::process::exit(1);
        }
        println!("\nAll gated streams identical. Run `ramverify` for the RA-RAM proof.");
    }

    // summary.json for downstream tooling
    let mut s = String::from("[\n");
    for (i, r) in results.iter().enumerate() {
        s.push_str(&format!(
            "  {{\"core\":\"{}\",\"game\":\"{}\",\"rep\":{},\"avg_fps\":{:.2},\"speed_x\":{:.3},\"p99_ms\":{:.3},\"max_ms\":{:.3},\"over_budget\":{},\"video_hash\":\"{}\",\"audio_hash\":\"{}\",\"ram_hash\":\"{}\",\"saveram_hash\":\"{}\"}}{}",
            r.core, r.game, r.rep, r.avg_fps, r.speed_x, r.p99, r.max, r.over_budget,
            r.video_hash, r.audio_hash, r.ram_hash, r.saveram_hash,
            if i + 1 == results.len() { "\n" } else { ",\n" }
        ));
    }
    s.push_str("]\n");
    std::fs::write(out_dir.join("summary.json"), s).expect("write summary");
    println!("\nresults saved to {}", out_dir.display());
}
