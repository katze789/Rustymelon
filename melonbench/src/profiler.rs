//! Flat sampling profiler for the core DLL.
//!
//! A sampler thread periodically suspends every other thread in this process,
//! reads its RIP, resumes it, and records the address. Samples are classified
//! as core-DLL code (reported as RVAs for offline symbolization with `nm`),
//! JIT-emitted code (anonymous executable pages), or other (host libs).
//!
//! Safety notes: between SuspendThread and ResumeThread we must not allocate
//! or take any lock the suspended thread might hold, so the sample buffer is
//! preallocated and only raw Win32 calls happen while a thread is suspended.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

type HANDLE = *mut c_void;

const TH32CS_SNAPTHREAD: u32 = 0x4;
const THREAD_SUSPEND_RESUME: u32 = 0x2;
const THREAD_GET_CONTEXT: u32 = 0x8;
const THREAD_QUERY_INFORMATION: u32 = 0x40;
const CONTEXT_CONTROL_AMD64: u32 = 0x0010_0001;
const RIP_OFFSET: usize = 0xF8;
const CONTEXT_FLAGS_OFFSET: usize = 0x30;
const MEM_COMMIT: u32 = 0x1000;
const PAGE_EXECUTE_ANY: u32 = 0x10 | 0x20 | 0x40 | 0x80;

#[repr(C)]
struct THREADENTRY32 {
    dw_size: u32,
    cnt_usage: u32,
    th32_thread_id: u32,
    th32_owner_process_id: u32,
    tp_base_pri: i32,
    tp_delta_pri: i32,
    dw_flags: u32,
}

#[repr(C, align(16))]
struct CONTEXT_BUF([u8; 1232]);

#[repr(C)]
struct MEMORY_BASIC_INFORMATION {
    base_address: *mut c_void,
    allocation_base: *mut c_void,
    allocation_protect: u32,
    partition_id: u16,
    region_size: usize,
    state: u32,
    protect: u32,
    mem_type: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> HANDLE;
    fn Thread32First(snap: HANDLE, entry: *mut THREADENTRY32) -> i32;
    fn Thread32Next(snap: HANDLE, entry: *mut THREADENTRY32) -> i32;
    fn CloseHandle(h: HANDLE) -> i32;
    fn OpenThread(access: u32, inherit: i32, tid: u32) -> HANDLE;
    fn SuspendThread(h: HANDLE) -> u32;
    fn ResumeThread(h: HANDLE) -> u32;
    fn GetThreadContext(h: HANDLE, ctx: *mut CONTEXT_BUF) -> i32;
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThreadId() -> u32;
    fn Sleep(ms: u32);
    fn VirtualQuery(
        addr: *const c_void,
        info: *mut MEMORY_BASIC_INFORMATION,
        len: usize,
    ) -> usize;
}

#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
    fn timeEndPeriod(period: u32) -> u32;
}

pub struct Sample {
    pub tid: u32,
    pub rip: u64,
}

pub struct Profiler {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Vec<Sample>>>,
}

fn enumerate_threads(self_tid: u32) -> Vec<(u32, HANDLE)> {
    let mut out = Vec::new();
    unsafe {
        let pid = GetCurrentProcessId();
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap.is_null() || snap as isize == -1 {
            return out;
        }
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dw_size = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut ok = Thread32First(snap, &mut entry);
        while ok != 0 {
            if entry.th32_owner_process_id == pid && entry.th32_thread_id != self_tid {
                let h = OpenThread(
                    THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION,
                    0,
                    entry.th32_thread_id,
                );
                if !h.is_null() {
                    out.push((entry.th32_thread_id, h));
                }
            }
            ok = Thread32Next(snap, &mut entry);
        }
        CloseHandle(snap);
    }
    out
}

impl Profiler {
    /// Begin sampling every `interval_ms` milliseconds (>= 1).
    pub fn start(interval_ms: u32) -> Profiler {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = std::thread::spawn(move || {
            unsafe { timeBeginPeriod(1) };
            let self_tid = unsafe { GetCurrentThreadId() };
            let mut samples: Vec<Sample> = Vec::with_capacity(4_000_000);
            let mut threads = enumerate_threads(self_tid);
            let mut iterations: u64 = 0;
            let mut ctx = CONTEXT_BUF([0u8; 1232]);
            while !stop2.load(Ordering::Relaxed) {
                iterations += 1;
                // Pick up newly spawned emulator threads now and then.
                if iterations % 2048 == 0 {
                    for (_, h) in threads.drain(..) {
                        unsafe { CloseHandle(h) };
                    }
                    threads = enumerate_threads(self_tid);
                }
                for &(tid, h) in &threads {
                    unsafe {
                        if SuspendThread(h) == u32::MAX {
                            continue;
                        }
                        // Set ContextFlags, fetch, read RIP. No allocation here.
                        let flags_ptr =
                            ctx.0.as_mut_ptr().add(CONTEXT_FLAGS_OFFSET) as *mut u32;
                        *flags_ptr = CONTEXT_CONTROL_AMD64;
                        let got = GetThreadContext(h, &mut ctx);
                        ResumeThread(h);
                        if got != 0 && samples.len() < samples.capacity() {
                            let rip =
                                *(ctx.0.as_ptr().add(RIP_OFFSET) as *const u64);
                            samples.push(Sample { tid, rip });
                        }
                    }
                }
                unsafe { Sleep(interval_ms) };
            }
            for (_, h) in threads {
                unsafe { CloseHandle(h) };
            }
            unsafe { timeEndPeriod(1) };
            samples
        });
        Profiler {
            stop,
            handle: Some(handle),
        }
    }

    pub fn stop(mut self) -> Vec<Sample> {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.take().map(|h| h.join().unwrap()).unwrap_or_default()
    }
}

/// Reads SizeOfImage out of the PE header the DLL was mapped at.
pub unsafe fn module_image_size(base: *mut c_void) -> usize {
    let base = base as *const u8;
    let e_lfanew = *(base.add(0x3C) as *const u32) as usize;
    // NT headers: signature(4) + file header(20), optional header follows.
    let optional = base.add(e_lfanew + 24);
    *(optional.add(0x38) as *const u32) as usize
}

/// True if the address belongs to a committed executable region that is
/// not backed by an image (i.e. JIT-emitted code).
pub fn is_jit_code(addr: u64) -> bool {
    unsafe {
        let mut info: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
        let got = VirtualQuery(
            addr as *const c_void,
            &mut info,
            std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
        );
        const MEM_IMAGE: u32 = 0x100_0000;
        got != 0
            && info.state == MEM_COMMIT
            && (info.protect & PAGE_EXECUTE_ANY) != 0
            && info.mem_type != MEM_IMAGE
    }
}
