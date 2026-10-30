use crate::{mem, ptrace};
use std::ffi::{c_void, CStr};
use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;

fn retry_find_func(pid: i32, name: &str) -> Result<u64, String> {
    for attempt in 0..20 {
        match find_func_remote(pid, name) {
            Ok(addr) => return Ok(addr),
            Err(_e) if attempt < 19 => {
                if attempt == 0 {
                    eprintln!("  waiting for {name} in target maps...");
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

#[repr(C)]
struct DlInfo {
    dli_fname: *const u8,
    dli_fbase: *mut c_void,
    dli_sname: *const u8,
    dli_saddr: *mut c_void,
}

extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const u8) -> *mut c_void;
    fn dladdr(addr: *mut c_void, info: *mut DlInfo) -> i32;
}

fn local_sym(name: &str) -> Result<u64, String> {
    let sym = format!("{name}\0");
    let ptr = unsafe { dlsym(std::ptr::null_mut(), sym.as_ptr()) };
    if ptr.is_null() {
        Err(format!("dlsym({name}) = NULL"))
    } else {
        Ok(ptr as u64)
    }
}

/// Find a function's address in a remote process.
/// 1. dlsym locally → get function address
/// 2. dladdr → get the library load base + filename
/// 3. offset = func_addr - load_base
/// 4. Find the first mapping of the same library in remote /proc/pid/maps
/// 5. remote_func = remote_first_mapping_start + offset
fn find_func_remote(pid: i32, name: &str) -> Result<u64, String> {
    let local_addr = local_sym(name)?;

    let mut info: DlInfo = unsafe { std::mem::zeroed() };
    let r = unsafe { dladdr(local_addr as *mut c_void, &mut info) };
    if r == 0 {
        return Err(format!("dladdr({name}) failed"));
    }

    let local_base = info.dli_fbase as u64;
    let offset = local_addr.wrapping_sub(local_base);

    let fname_cstr = unsafe { CStr::from_ptr(info.dli_fname as *const i8) };
    let fname = fname_cstr.to_string_lossy().to_string();
    let lib_name = Path::new(&fname)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&fname)
        .to_string();


    let remote_maps = fs::read_to_string(format!("/proc/{pid}/maps"))
        .map_err(|e| format!("remote maps: {e}"))?;

    for line in remote_maps.lines() {
        if line.contains(lib_name.as_str()) {
            let range = line.split_whitespace().next().unwrap_or("");
            let start_str = range.split('-').next().unwrap_or("");
            let remote_base = u64::from_str_radix(start_str, 16)
                .map_err(|e| format!("bad addr: {e}"))?;
            return Ok(remote_base.wrapping_add(offset));
        }
    }

    Err(format!("{lib_name} not found in remote maps"))
}

/// Inject a .so into a remote process via ptrace + dlopen.
///
/// Pipeline:
/// 1. PTRACE_ATTACH
/// 2. Find mmap in target (offset from libc)
/// 3. call_remote(mmap, ...) → allocate writable page
/// 4. process_vm_writev the .so path into the page
/// 5. Find dlopen in target
/// 6. call_remote(dlopen, path, RTLD_NOW) → .so loads, constructors run
/// 7. Detach (RAII)
/// Stage the .so into the target's own filesystem so dlopen resolves it
/// regardless of mount namespace (Flatpak/Sober runs in a bwrap sandbox).
/// Returns the path *as seen by the target*.
fn stage_so(pid: i32, so_path: &str) -> Result<String, String> {
    let src = Path::new(so_path);
    if !src.exists() {
        return Err(format!("{so_path} does not exist"));
    }
    let basename = src
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("bad .so filename: {so_path}"))?;

    let target_path = format!("/proc/{pid}/root/tmp/{basename}");
    match fs::copy(src, &target_path) {
        Ok(_) => eprintln!("  staged  -> {target_path}"),
        Err(e) => eprintln!("  stage   skipped ({e}), using original path"),
    }
    Ok(format!("/tmp/{basename}"))
}

pub fn inject_so(pid: i32, so_path: &str, start_symbol: Option<&str>) -> Result<(), String> {
    // Find function addresses BEFORE attaching.
    // The target must be running so ld.so can finish loading libc.
    let mmap_addr = retry_find_func(pid, "mmap")?;
    eprintln!("  mmap    @ {mmap_addr:#x}");

    let dlopen_addr = retry_find_func(pid, "dlopen")?;
    eprintln!("  dlopen  @ {dlopen_addr:#x}");

    let dlsym_addr = retry_find_func(pid, "dlsym")?;
    eprintln!("  dlsym   @ {dlsym_addr:#x}");

    // Path as the target will see it (staged into its own /tmp)
    let remote_path = stage_so(pid, so_path)?;

    // NOW attach — target is stopped from here until guard drops
    let _guard = ptrace::attach(pid)?;

    let remote_buf = ptrace::call_remote(pid, mmap_addr, &[
        0,           // addr = NULL
        4096,        // len
        3,           // PROT_READ | PROT_WRITE
        0x22,        // MAP_PRIVATE | MAP_ANONYMOUS
        !0u64,       // fd = -1
        0,           // offset
    ])?;

    if remote_buf == !0u64 {
        return Err("mmap returned MAP_FAILED".into());
    }
    eprintln!("  alloc   @ {remote_buf:#x}");

    // Layout in the allocated page:
    //   [0x000] .so path  (NUL-terminated)
    //   [0x200] start symbol name (NUL-terminated)
    let path_c = format!("{remote_path}\0");
    mem::write_mem(pid, remote_buf, path_c.as_bytes())?;

    let sym = start_symbol.unwrap_or("solara_payload_start");
    let sym_c = format!("{sym}\0");
    let sym_addr = remote_buf + 0x200;
    mem::write_mem(pid, sym_addr, sym_c.as_bytes())?;

    // dlopen(path, RTLD_NOW)
    let handle = ptrace::call_remote(pid, dlopen_addr, &[remote_buf, 2])?;
    if handle == 0 {
        return Err("dlopen returned NULL (check .so path is valid in target's mount ns)".into());
    }
    eprintln!("  loaded  handle={handle:#x}");

    // dlsym(handle, sym)
    let fn_addr = ptrace::call_remote(pid, dlsym_addr, &[handle, sym_addr])?;
    if fn_addr == 0 {
        return Err(format!("dlsym({sym}) returned NULL — symbol not exported?"));
    }
    eprintln!("  {sym} @ {fn_addr:#x}");

    // Call the exported entry point
    ptrace::call_remote(pid, fn_addr, &[])?;
    eprintln!("  started");

    Ok(())
}