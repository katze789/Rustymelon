//! melonbench — headless libretro benchmark & behavior-verification harness.
//!
//! Loads a libretro core DLL, runs a ROM for N frames with no video/audio
//! output, and reports wall-clock frame-time statistics plus rolling hashes
//! of the video and audio streams so two cores (or two builds of the same
//! core) can be compared for both speed and identical behavior.
//!
//! No part of this touches the real RetroArch install: system and save
//! directories are whatever sandbox paths are passed on the command line.

#![allow(static_mut_refs)]

mod libretro;
mod profiler;
mod ramverify;
mod suite;
mod symbolize;

use libretro::*;
use std::collections::HashMap;
use std::ffi::{c_void, CStr, CString};
use std::os::raw::c_char;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::time::Instant;

// ---------------------------------------------------------------------------
// kernel32 bindings (no external crates, keeps the toolchain footprint small)
// ---------------------------------------------------------------------------

type HMODULE = *mut c_void;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> HMODULE;
    fn GetProcAddress(h: HMODULE, name: *const u8) -> *mut c_void;
    fn GetLastError() -> u32;
    fn GetCurrentProcess() -> HMODULE;
    fn SetPriorityClass(process: HMODULE, class: u32) -> i32;
}

const HIGH_PRIORITY_CLASS: u32 = 0x0000_0080;

fn load_library(path: &Path) -> HMODULE {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe { LoadLibraryW(wide.as_ptr()) }
}

unsafe fn sym(h: HMODULE, name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap();
    let p = GetProcAddress(h, c.as_ptr() as *const u8);
    if p.is_null() {
        panic!("core is missing required symbol {name}");
    }
    p
}

// ---------------------------------------------------------------------------
// Harness state shared with the libretro callbacks.
// The harness is single-threaded: every callback below is invoked from inside
// retro_run() on this thread (the core's internal render thread never calls
// frontend callbacks), so a static mut is safe in practice.
// ---------------------------------------------------------------------------

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[inline]
fn fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

struct State {
    options: HashMap<String, CString>,
    system_dir: CString,
    save_dir: CString,
    pixel_format: u32,
    frame: u64,
    hash_streams: bool,
    video_hash: u64,
    audio_hash: u64,
    // Rolling hashes of the emulated memory RetroAchievements reads. These are
    // the bytes that determine whether achievements unlock; a behavior-
    // preserving change MUST leave them byte-identical frame by frame.
    ram_hash: u64,
    saveram_hash: u64,
    audio_samples: u64,
    dupe_frames: u64,
    // (first_frame, last_frame, button_id) — held inclusive on both ends
    input_schedule: Vec<(u64, u64, u32)>,
    shutdown: bool,
    messages: Vec<String>,
    verbose_env: bool,
}

impl State {
    fn new() -> Self {
        State {
            options: HashMap::new(),
            system_dir: CString::new("").unwrap(),
            save_dir: CString::new("").unwrap(),
            pixel_format: RETRO_PIXEL_FORMAT_0RGB1555,
            frame: 0,
            hash_streams: false,
            video_hash: FNV_OFFSET,
            audio_hash: FNV_OFFSET,
            ram_hash: FNV_OFFSET,
            saveram_hash: FNV_OFFSET,
            audio_samples: 0,
            dupe_frames: 0,
            input_schedule: Vec::new(),
            shutdown: false,
            messages: Vec::new(),
            verbose_env: false,
        }
    }
}

static mut STATE: Option<State> = None;

fn state() -> &'static mut State {
    unsafe { STATE.as_mut().expect("state initialized") }
}

// ---------------------------------------------------------------------------
// libretro callbacks
// ---------------------------------------------------------------------------

unsafe extern "C" fn env_cb(cmd: u32, data: *mut c_void) -> bool {
    let st = state();
    let plain = cmd & !RETRO_ENVIRONMENT_EXPERIMENTAL;
    match plain {
        ENV_GET_CAN_DUPE => {
            *(data as *mut bool) = true;
            true
        }
        ENV_GET_SYSTEM_DIRECTORY => {
            *(data as *mut *const c_char) = st.system_dir.as_ptr();
            true
        }
        ENV_GET_SAVE_DIRECTORY => {
            *(data as *mut *const c_char) = st.save_dir.as_ptr();
            true
        }
        ENV_SET_PIXEL_FORMAT => {
            st.pixel_format = *(data as *const u32);
            true
        }
        ENV_GET_VARIABLE => {
            let var = &mut *(data as *mut retro_variable);
            if var.key.is_null() {
                return false;
            }
            let key = CStr::from_ptr(var.key).to_string_lossy().into_owned();
            match st.options.get(&key) {
                Some(v) => {
                    var.value = v.as_ptr();
                    true
                }
                None => false, // core falls back to its own default
            }
        }
        ENV_GET_VARIABLE_UPDATE => {
            *(data as *mut bool) = false;
            true
        }
        ENV_SET_VARIABLES
        | ENV_SET_CORE_OPTIONS
        | ENV_SET_CORE_OPTIONS_INTL
        | ENV_SET_CORE_OPTIONS_DISPLAY
        | ENV_SET_CORE_OPTIONS_V2
        | ENV_SET_CORE_OPTIONS_V2_INTL
        | ENV_SET_CORE_OPTIONS_UPDATE_DISPLAY_CALLBACK
        | ENV_SET_INPUT_DESCRIPTORS
        | ENV_SET_CONTROLLER_INFO
        | ENV_SET_SUPPORT_ACHIEVEMENTS
        | ENV_SET_CONTENT_INFO_OVERRIDE
        | ENV_SET_GEOMETRY
        | ENV_SET_SYSTEM_AV_INFO => true,
        ENV_GET_CORE_OPTIONS_VERSION => {
            *(data as *mut u32) = 2;
            true
        }
        ENV_GET_LANGUAGE => {
            *(data as *mut u32) = 0; // English
            true
        }
        ENV_GET_AUDIO_VIDEO_ENABLE => {
            *(data as *mut i32) = 3; // video + audio both wanted
            true
        }
        ENV_GET_JIT_CAPABLE => {
            *(data as *mut bool) = true;
            true
        }
        ENV_SET_MESSAGE => {
            let msg = &*(data as *const retro_message);
            if !msg.msg.is_null() {
                let s = CStr::from_ptr(msg.msg).to_string_lossy().into_owned();
                eprintln!("[core message] {s}");
                st.messages.push(s);
            }
            true
        }
        ENV_SET_MESSAGE_EXT => {
            // First field of retro_message_ext is the msg pointer.
            let msg = *(data as *const *const c_char);
            if !msg.is_null() {
                let s = CStr::from_ptr(msg).to_string_lossy().into_owned();
                eprintln!("[core message] {s}");
                st.messages.push(s);
            }
            true
        }
        ENV_SHUTDOWN => {
            st.shutdown = true;
            true
        }
        // Declined on purpose: SET_HW_RENDER (forces the software renderer,
        // matching the user's real settings), GET_LOG_INTERFACE (core falls
        // back to stderr), microphone/rumble/perf/VFS interfaces, etc.
        _ => {
            if st.verbose_env {
                eprintln!("[env] declined cmd {plain} (raw {cmd})");
            }
            false
        }
    }
}

unsafe extern "C" fn video_cb(data: *const c_void, width: u32, height: u32, pitch: usize) {
    let st = state();
    if data.is_null() {
        st.dupe_frames += 1;
        return;
    }
    if st.hash_streams {
        let bpp = match st.pixel_format {
            RETRO_PIXEL_FORMAT_XRGB8888 => 4,
            _ => 2,
        };
        let row_bytes = (width as usize) * bpp;
        let mut h = st.video_hash;
        for y in 0..height as usize {
            let row = std::slice::from_raw_parts((data as *const u8).add(y * pitch), row_bytes);
            h = fnv1a(h, row);
        }
        st.video_hash = h;
    }
}

unsafe extern "C" fn audio_sample_cb(left: i16, right: i16) {
    let st = state();
    st.audio_samples += 1;
    if st.hash_streams {
        st.audio_hash = fnv1a(st.audio_hash, &left.to_le_bytes());
        st.audio_hash = fnv1a(st.audio_hash, &right.to_le_bytes());
    }
}

unsafe extern "C" fn audio_batch_cb(data: *const i16, frames: usize) -> usize {
    let st = state();
    st.audio_samples += frames as u64;
    if st.hash_streams && !data.is_null() {
        let bytes = std::slice::from_raw_parts(data as *const u8, frames * 4);
        st.audio_hash = fnv1a(st.audio_hash, bytes);
    }
    frames
}

unsafe extern "C" fn input_poll_cb() {}

unsafe extern "C" fn input_state_cb(port: u32, device: u32, _index: u32, id: u32) -> i16 {
    if port != 0 || device != RETRO_DEVICE_JOYPAD {
        return 0;
    }
    let st = state();
    for &(start, end, button) in &st.input_schedule {
        if button == id && st.frame >= start && st.frame <= end {
            return 1;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

struct Args {
    core: String,
    rom: String,
    system_dir: String,
    save_dir: String,
    opt_file: Option<String>,
    overrides: Vec<(String, String)>,
    frames: u64,
    warmup: u64,
    hash_streams: bool,
    verify: bool,
    json_out: Option<String>,
    csv_out: Option<String>,
    label: String,
    inputs: Vec<(u64, u64, u32)>,
    verbose_env: bool,
    profile_out: Option<String>,
    profile_interval_ms: u32,
    ram_dump: Option<String>,
}

fn usage() -> ! {
    eprintln!(
        "usage: melonbench --core CORE.dll --rom GAME.nds --system-dir DIR [options]

options:
  --save-dir DIR        save directory (default: <system-dir>\\..\\saves)
  --opt FILE            RetroArch .opt file to source core options from
  --set KEY=VALUE       override a single core option (repeatable)
  --frames N            measured frames (default 3000)
  --warmup N            unmeasured warmup frames (default 300)
  --hash                hash video+audio streams (for behavior comparison)
  --verify              deterministic mode: implies --hash and forces
                        start_time_mode=absolute, mic_input=silence
  --input BTN@START-END hold joypad BTN (A/B/X/Y/START/...) over a frame range
                        (repeatable, e.g. --input START@400-410)
  --json FILE           write results as JSON
  --csv FILE            write per-frame times (ms) as CSV
  --label NAME          label for this run in output
  --profile FILE        sample all threads during the measured phase and
                        write a flat profile (core-DLL RVAs + JIT/other)
  --profile-ms N        sampling period in ms (default 1)
  --ram-dump FILE       write final SYSTEM_RAM snapshot (for `ramverify`)
  --verbose-env         log declined environment calls"
    );
    std::process::exit(2);
}

fn parse_input_spec(spec: &str) -> Option<(u64, u64, u32)> {
    let (btn, range) = spec.split_once('@')?;
    let (a, b) = range.split_once('-')?;
    let id = JOYPAD_BUTTONS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(btn))
        .map(|(_, i)| *i)?;
    Some((a.parse().ok()?, b.parse().ok()?, id))
}

fn parse_args() -> Args {
    let mut a = Args {
        core: String::new(),
        rom: String::new(),
        system_dir: String::new(),
        save_dir: String::new(),
        opt_file: None,
        overrides: Vec::new(),
        frames: 3000,
        warmup: 300,
        hash_streams: false,
        verify: false,
        json_out: None,
        csv_out: None,
        label: String::new(),
        inputs: Vec::new(),
        verbose_env: false,
        profile_out: None,
        profile_interval_ms: 1,
        ram_dump: None,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    let next = |i: &mut usize| -> String {
        *i += 1;
        argv.get(*i).cloned().unwrap_or_else(|| usage())
    };
    while i < argv.len() {
        match argv[i].as_str() {
            "--core" => a.core = next(&mut i),
            "--rom" => a.rom = next(&mut i),
            "--system-dir" => a.system_dir = next(&mut i),
            "--save-dir" => a.save_dir = next(&mut i),
            "--opt" => a.opt_file = Some(next(&mut i)),
            "--set" => {
                let kv = next(&mut i);
                match kv.split_once('=') {
                    Some((k, v)) => a.overrides.push((k.to_string(), v.to_string())),
                    None => usage(),
                }
            }
            "--frames" => a.frames = next(&mut i).parse().unwrap_or_else(|_| usage()),
            "--warmup" => a.warmup = next(&mut i).parse().unwrap_or_else(|_| usage()),
            "--hash" => a.hash_streams = true,
            "--verify" => {
                a.verify = true;
                a.hash_streams = true;
            }
            "--input" => {
                let spec = next(&mut i);
                a.inputs.push(parse_input_spec(&spec).unwrap_or_else(|| usage()));
            }
            "--json" => a.json_out = Some(next(&mut i)),
            "--csv" => a.csv_out = Some(next(&mut i)),
            "--label" => a.label = next(&mut i),
            "--profile" => a.profile_out = Some(next(&mut i)),
            "--profile-ms" => {
                a.profile_interval_ms = next(&mut i).parse().unwrap_or_else(|_| usage())
            }
            "--ram-dump" => a.ram_dump = Some(next(&mut i)),
            "--verbose-env" => a.verbose_env = true,
            _ => usage(),
        }
        i += 1;
    }
    if a.core.is_empty() || a.rom.is_empty() || a.system_dir.is_empty() {
        usage();
    }
    if a.save_dir.is_empty() {
        a.save_dir = format!("{}\\..\\saves", a.system_dir);
    }
    if a.label.is_empty() {
        a.label = Path::new(&a.core)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "core".into());
    }
    a
}

fn load_opt_file(path: &str) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read option file {path}: {e}"));
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            let v = v.trim().trim_matches('"');
            out.push((k.to_string(), v.to_string()));
        }
    }
    out
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p / 100.0).round() as usize;
    sorted[idx]
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// ---------------------------------------------------------------------------

fn main() {
    // Subcommands: `suite` (cores x games x reps) and `symbolize` (profile ->
    // function names). Anything else is the original single-run flag CLI.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match argv.first().map(String::as_str) {
        Some("suite") => return suite::run_suite(&argv[1..]),
        Some("symbolize") => return symbolize::run_symbolize(&argv[1..]),
        Some("ramverify") => return ramverify::run_ramverify(&argv[1..]),
        _ => {}
    }

    // Reduce scheduling noise from background processes during measurement.
    unsafe { SetPriorityClass(GetCurrentProcess(), HIGH_PRIORITY_CLASS) };

    let args = parse_args();

    // Assemble core options: .opt file first, then explicit --set overrides,
    // then verify-mode determinism overrides on top.
    let mut opts: Vec<(String, String)> = Vec::new();
    if let Some(f) = &args.opt_file {
        opts.extend(load_opt_file(f));
    }
    opts.extend(args.overrides.clone());
    if args.verify {
        opts.push(("melonds_start_time_mode".into(), "absolute".into()));
        opts.push(("melonds_mic_input".into(), "silence".into()));
    }

    let mut st = State::new();
    for (k, v) in &opts {
        st.options
            .insert(k.clone(), CString::new(v.as_str()).unwrap());
    }
    st.system_dir = CString::new(args.system_dir.as_str()).unwrap();
    st.save_dir = CString::new(args.save_dir.as_str()).unwrap();
    st.hash_streams = args.hash_streams;
    st.input_schedule = args.inputs.clone();
    st.verbose_env = args.verbose_env;
    unsafe { STATE = Some(st) };

    std::fs::create_dir_all(&args.save_dir).ok();

    // Load the core and resolve entry points.
    let h = load_library(Path::new(&args.core));
    if h.is_null() {
        eprintln!(
            "failed to load core {} (Win32 error {})",
            args.core,
            unsafe { GetLastError() }
        );
        std::process::exit(1);
    }

    unsafe {
        let api_version: retro_api_version_t = std::mem::transmute(sym(h, "retro_api_version"));
        assert_eq!(api_version(), RETRO_API_VERSION, "unsupported libretro API");

        let set_environment: retro_set_environment_t =
            std::mem::transmute(sym(h, "retro_set_environment"));
        let set_video: retro_set_video_refresh_t =
            std::mem::transmute(sym(h, "retro_set_video_refresh"));
        let set_audio: retro_set_audio_sample_t =
            std::mem::transmute(sym(h, "retro_set_audio_sample"));
        let set_audio_batch: retro_set_audio_sample_batch_t =
            std::mem::transmute(sym(h, "retro_set_audio_sample_batch"));
        let set_input_poll: retro_set_input_poll_t =
            std::mem::transmute(sym(h, "retro_set_input_poll"));
        let set_input_state: retro_set_input_state_t =
            std::mem::transmute(sym(h, "retro_set_input_state"));
        let init: retro_init_t = std::mem::transmute(sym(h, "retro_init"));
        let deinit: retro_deinit_t = std::mem::transmute(sym(h, "retro_deinit"));
        let get_system_info: retro_get_system_info_t =
            std::mem::transmute(sym(h, "retro_get_system_info"));
        let get_av_info: retro_get_system_av_info_t =
            std::mem::transmute(sym(h, "retro_get_system_av_info"));
        let load_game: retro_load_game_t = std::mem::transmute(sym(h, "retro_load_game"));
        let unload_game: retro_unload_game_t = std::mem::transmute(sym(h, "retro_unload_game"));
        let run: retro_run_t = std::mem::transmute(sym(h, "retro_run"));
        let get_memory_data: retro_get_memory_data_t =
            std::mem::transmute(sym(h, "retro_get_memory_data"));
        let get_memory_size: retro_get_memory_size_t =
            std::mem::transmute(sym(h, "retro_get_memory_size"));

        set_environment(env_cb);
        init();
        set_video(video_cb);
        set_audio(audio_sample_cb);
        set_audio_batch(audio_batch_cb);
        set_input_poll(input_poll_cb);
        set_input_state(input_state_cb);

        let mut sysinfo: retro_system_info = std::mem::zeroed();
        get_system_info(&mut sysinfo);
        let core_name = CStr::from_ptr(sysinfo.library_name).to_string_lossy().into_owned();
        let core_version = CStr::from_ptr(sysinfo.library_version).to_string_lossy().into_owned();
        eprintln!("core: {core_name} {core_version}");

        let rom_path_c = CString::new(args.rom.as_str()).unwrap();
        let rom_data = if sysinfo.need_fullpath {
            Vec::new()
        } else {
            std::fs::read(&args.rom).unwrap_or_else(|e| panic!("cannot read ROM: {e}"))
        };
        let game = retro_game_info {
            path: rom_path_c.as_ptr(),
            data: if rom_data.is_empty() {
                std::ptr::null()
            } else {
                rom_data.as_ptr() as *const c_void
            },
            size: rom_data.len(),
            meta: std::ptr::null(),
        };

        if !load_game(&game) {
            eprintln!("retro_load_game failed");
            std::process::exit(1);
        }

        let mut av: retro_system_av_info = std::mem::zeroed();
        get_av_info(&mut av);
        let content_fps = av.timing.fps;
        eprintln!(
            "av info: {}x{} @ {:.4} fps, {} Hz audio",
            av.geometry.base_width, av.geometry.base_height, content_fps, av.timing.sample_rate
        );

        // Resolve the memory regions RetroAchievements reads (stable pointers).
        let ram_ptr = get_memory_data(RETRO_MEMORY_SYSTEM_RAM) as *const u8;
        let ram_size = get_memory_size(RETRO_MEMORY_SYSTEM_RAM);
        let saveram_ptr = get_memory_data(RETRO_MEMORY_SAVE_RAM) as *const u8;
        let saveram_size = get_memory_size(RETRO_MEMORY_SAVE_RAM);
        if args.hash_streams {
            eprintln!(
                "memory: system_ram {} bytes, save_ram {} bytes",
                ram_size, saveram_size
            );
        }

        // Warmup, then measured run.
        let total = args.warmup + args.frames;
        let mut times_ms: Vec<f64> = Vec::with_capacity(args.frames as usize);
        let wall_start = Instant::now();
        let mut measured_start = None;
        let mut prof: Option<profiler::Profiler> = None;
        for i in 0..total {
            state().frame = i;
            let t0 = Instant::now();
            run();
            let dt = t0.elapsed().as_secs_f64() * 1000.0;
            // Fold the RA-visible memory into a rolling hash every frame, so a
            // divergence on any single frame is caught (not just the last).
            if args.hash_streams {
                let st = state();
                if !ram_ptr.is_null() && ram_size > 0 {
                    let mem = std::slice::from_raw_parts(ram_ptr, ram_size);
                    st.ram_hash = fnv1a(st.ram_hash, mem);
                }
                if !saveram_ptr.is_null() && saveram_size > 0 {
                    let mem = std::slice::from_raw_parts(saveram_ptr, saveram_size);
                    st.saveram_hash = fnv1a(st.saveram_hash, mem);
                }
            }
            if i >= args.warmup {
                if measured_start.is_none() {
                    measured_start = Some(Instant::now());
                    if args.profile_out.is_some() {
                        prof = Some(profiler::Profiler::start(args.profile_interval_ms));
                    }
                }
                times_ms.push(dt);
            }
            if state().shutdown {
                eprintln!("core requested shutdown at frame {i}");
                break;
            }
        }
        let samples = prof.map(|p| p.stop()).unwrap_or_default();

        if let Some(dump) = &args.ram_dump {
            if !ram_ptr.is_null() && ram_size > 0 {
                let mem = std::slice::from_raw_parts(ram_ptr, ram_size);
                std::fs::write(dump, mem).expect("write ram dump");
                eprintln!("ram snapshot: {} bytes -> {}", ram_size, dump);
            }
        }

        let measured_wall = measured_start.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0);
        let wall_total = wall_start.elapsed().as_secs_f64();

        let st = state();
        let frames_measured = times_ms.len() as u64;
        let mut sorted = times_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mean_ms: f64 = if times_ms.is_empty() {
            0.0
        } else {
            times_ms.iter().sum::<f64>() / times_ms.len() as f64
        };
        let avg_fps = if measured_wall > 0.0 {
            frames_measured as f64 / measured_wall
        } else {
            0.0
        };
        let speed_x = if content_fps > 0.0 { avg_fps / content_fps } else { 0.0 };
        let budget_ms = 1000.0 / content_fps;
        let spikes = times_ms.iter().filter(|&&t| t > budget_ms).count();

        let p50 = percentile(&sorted, 50.0);
        let p90 = percentile(&sorted, 90.0);
        let p99 = percentile(&sorted, 99.0);
        let max = sorted.last().copied().unwrap_or(0.0);

        println!();
        println!("=== {} ===", args.label);
        println!("core:            {core_name} {core_version}");
        println!("rom:             {}", args.rom);
        println!("frames measured: {frames_measured} (warmup {})", args.warmup);
        println!("avg fps:         {avg_fps:.2}  ({speed_x:.2}x realtime)");
        println!("frame ms:        mean {mean_ms:.3}  p50 {p50:.3}  p90 {p90:.3}  p99 {p99:.3}  max {max:.3}");
        println!(
            "over budget:     {spikes} frames > {budget_ms:.2} ms ({:.2}%)",
            100.0 * spikes as f64 / frames_measured.max(1) as f64
        );
        println!("dupe frames:     {}", st.dupe_frames);
        println!("audio samples:   {}", st.audio_samples);
        if st.hash_streams {
            println!("video hash:      {:016x}", st.video_hash);
            println!("audio hash:      {:016x}", st.audio_hash);
            println!("ram hash:        {:016x}  (RetroAchievements-visible)", st.ram_hash);
            println!("saveram hash:    {:016x}", st.saveram_hash);
        }
        println!("wall time:       {wall_total:.1} s");

        if let Some(pp) = &args.profile_out {
            // Classify samples: inside the core DLL (-> RVA), JIT code, other.
            let dll_base = h as u64;
            let dll_size = profiler::module_image_size(h) as u64;
            let mut rva_counts: HashMap<u64, u64> = HashMap::new();
            let mut per_thread: HashMap<u32, u64> = HashMap::new();
            let (mut jit, mut other) = (0u64, 0u64);
            for s in &samples {
                *per_thread.entry(s.tid).or_default() += 1;
                if s.rip >= dll_base && s.rip < dll_base + dll_size {
                    *rva_counts.entry(s.rip - dll_base).or_default() += 1;
                } else if profiler::is_jit_code(s.rip) {
                    jit += 1;
                } else {
                    other += 1;
                }
            }
            let in_dll: u64 = rva_counts.values().sum();
            let mut out = String::new();
            out.push_str(&format!(
                "# melonbench flat profile  label={} core={}\n",
                args.label, args.core
            ));
            out.push_str(&format!(
                "# total={} in_dll={} jit={} other={} interval_ms={}\n",
                samples.len(),
                in_dll,
                jit,
                other,
                args.profile_interval_ms
            ));
            out.push_str("# threads: ");
            let mut tids: Vec<_> = per_thread.iter().collect();
            tids.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
            for (tid, c) in &tids {
                out.push_str(&format!("{tid}={c} "));
            }
            out.push_str("\n# rva,count (symbolize with nm + ImageBase)\n");
            let mut rvas: Vec<_> = rva_counts.into_iter().collect();
            rvas.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
            for (rva, c) in rvas {
                out.push_str(&format!("{rva:x},{c}\n"));
            }
            std::fs::write(pp, out).expect("write profile");
            eprintln!(
                "profile: {} samples ({} dll, {} jit, {} other) -> {}",
                samples.len(),
                in_dll,
                jit,
                other,
                pp
            );
        }

        if let Some(csv) = &args.csv_out {
            let mut s = String::from("frame,ms\n");
            for (i, t) in times_ms.iter().enumerate() {
                s.push_str(&format!("{i},{t:.4}\n"));
            }
            std::fs::write(csv, s).expect("write csv");
        }

        if let Some(jp) = &args.json_out {
            let opts_json: Vec<String> = opts
                .iter()
                .map(|(k, v)| format!("\"{}\":\"{}\"", json_escape(k), json_escape(v)))
                .collect();
            let json = format!(
                concat!(
                    "{{\n",
                    "  \"label\": \"{label}\",\n",
                    "  \"core_path\": \"{core_path}\",\n",
                    "  \"core_name\": \"{core_name}\",\n",
                    "  \"core_version\": \"{core_version}\",\n",
                    "  \"rom\": \"{rom}\",\n",
                    "  \"frames_measured\": {frames},\n",
                    "  \"warmup\": {warmup},\n",
                    "  \"content_fps\": {content_fps},\n",
                    "  \"avg_fps\": {avg_fps:.4},\n",
                    "  \"speed_x\": {speed_x:.4},\n",
                    "  \"frame_ms\": {{\"mean\": {mean:.4}, \"p50\": {p50:.4}, \"p90\": {p90:.4}, \"p99\": {p99:.4}, \"max\": {max:.4}}},\n",
                    "  \"frames_over_budget\": {spikes},\n",
                    "  \"dupe_frames\": {dupes},\n",
                    "  \"audio_samples\": {audio_samples},\n",
                    "  \"video_hash\": \"{vhash:016x}\",\n",
                    "  \"audio_hash\": \"{ahash:016x}\",\n",
                    "  \"ram_hash\": \"{ramhash:016x}\",\n",
                    "  \"saveram_hash\": \"{sramhash:016x}\",\n",
                    "  \"hashed\": {hashed},\n",
                    "  \"options\": {{{opts}}}\n",
                    "}}\n"
                ),
                label = json_escape(&args.label),
                core_path = json_escape(&args.core),
                core_name = json_escape(&core_name),
                core_version = json_escape(&core_version),
                rom = json_escape(&args.rom),
                frames = frames_measured,
                warmup = args.warmup,
                content_fps = content_fps,
                avg_fps = avg_fps,
                speed_x = speed_x,
                mean = mean_ms,
                p50 = p50,
                p90 = p90,
                p99 = p99,
                max = max,
                spikes = spikes,
                dupes = st.dupe_frames,
                audio_samples = st.audio_samples,
                vhash = st.video_hash,
                ahash = st.audio_hash,
                ramhash = st.ram_hash,
                sramhash = st.saveram_hash,
                hashed = st.hash_streams,
                opts = opts_json.join(",")
            );
            std::fs::write(jp, json).expect("write json");
        }

        unload_game();
        deinit();
        // Skip FreeLibrary: the core's worker threads may still be winding
        // down and the process is exiting anyway.
        std::process::exit(0);
    }
}
