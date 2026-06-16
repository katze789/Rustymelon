//! `melonbench symbolize` — resolve a melonbench flat profile's RVAs to
//! function names using binutils (nm/objdump) from the MinGW toolchain.
//! Rust port of the former symbolize-profile.ps1.

use std::collections::HashMap;
use std::process::Command;

struct SymArgs {
    profile: String,
    dll: String,
    top: usize,
    bin_dir: String,
}

fn sym_usage() -> ! {
    eprintln!(
        "usage: melonbench symbolize --profile FILE --dll CORE.dll [--top N] [--bin-dir DIR]

  --bin-dir DIR   directory containing nm.exe/objdump.exe
                  (default C:\\msys64\\mingw64\\bin)"
    );
    std::process::exit(2);
}

fn parse_sym_args(argv: &[String]) -> SymArgs {
    let mut a = SymArgs {
        profile: String::new(),
        dll: String::new(),
        top: 40,
        bin_dir: "C:\\msys64\\mingw64\\bin".into(),
    };
    let mut i = 0;
    let next = |i: &mut usize| -> String {
        *i += 1;
        argv.get(*i).cloned().unwrap_or_else(|| sym_usage())
    };
    while i < argv.len() {
        match argv[i].as_str() {
            "--profile" => a.profile = next(&mut i),
            "--dll" => a.dll = next(&mut i),
            "--top" => a.top = next(&mut i).parse().unwrap_or_else(|_| sym_usage()),
            "--bin-dir" => a.bin_dir = next(&mut i),
            _ => sym_usage(),
        }
        i += 1;
    }
    if a.profile.is_empty() || a.dll.is_empty() {
        sym_usage();
    }
    a
}

fn run_tool(bin_dir: &str, tool: &str, args: &[&str]) -> String {
    let path = format!("{bin_dir}\\{tool}");
    let out = Command::new(&path)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("cannot run {path}: {e}"));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn run_symbolize(argv: &[String]) {
    let args = parse_sym_args(argv);

    // Preferred load address the symbol table is relative to.
    let pe = run_tool(&args.bin_dir, "objdump.exe", &["-p", &args.dll]);
    let image_base = pe
        .lines()
        .find(|l| l.contains("ImageBase"))
        .and_then(|l| l.split_whitespace().last())
        .and_then(|h| u64::from_str_radix(h, 16).ok())
        .unwrap_or_else(|| panic!("no ImageBase in objdump output"));
    eprintln!("ImageBase: 0x{image_base:x}");

    // Sorted text-symbol table: rva -> name.
    let nm = run_tool(&args.bin_dir, "nm.exe", &["-n", "-C", "--defined-only", &args.dll]);
    let mut syms: Vec<(u64, &str)> = Vec::new();
    for line in nm.lines() {
        let mut parts = line.splitn(3, ' ');
        let (Some(addr), Some(kind), Some(name)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if kind != "t" && kind != "T" {
            continue;
        }
        if let Ok(va) = u64::from_str_radix(addr, 16) {
            if va >= image_base {
                syms.push((va - image_base, name));
            }
        }
    }
    syms.sort_by_key(|&(rva, _)| rva);
    eprintln!("text symbols: {}", syms.len());

    let resolve = |rva: u64| -> &str {
        match syms.binary_search_by_key(&rva, |&(r, _)| r) {
            Ok(i) => syms[i].1,
            Err(0) => "<unknown>",
            Err(i) => syms[i - 1].1,
        }
    };

    // Aggregate profile counts per enclosing symbol.
    let profile = std::fs::read_to_string(&args.profile)
        .unwrap_or_else(|e| panic!("cannot read profile {}: {e}", args.profile));
    let mut counts: HashMap<&str, u64> = HashMap::new();
    let mut total: u64 = 0;
    for line in profile.lines() {
        if line.starts_with('#') {
            println!("{line}");
            continue;
        }
        let Some((rva_hex, count)) = line.split_once(',') else {
            continue;
        };
        let (Ok(rva), Ok(c)) = (u64::from_str_radix(rva_hex, 16), count.parse::<u64>()) else {
            continue;
        };
        *counts.entry(resolve(rva)).or_default() += c;
        total += c;
    }

    println!("\n=== Hot functions ({total} in-DLL samples) ===");
    let mut sorted: Vec<_> = counts.into_iter().collect();
    sorted.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
    for (name, c) in sorted.into_iter().take(args.top) {
        println!("{:>7.2}%  {:>6}  {}", 100.0 * c as f64 / total.max(1) as f64, c, name);
    }
}
