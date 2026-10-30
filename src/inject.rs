use crate::{mem, ptrace};
use std::ffi::c_void;
use std::fs;
use std::path::Path;

extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const u8) -> *mut c_void;
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
/// Uses dlsym locally + libc offset math: remote = remote_base + (local - local_base)
fn find_func_remote(pid: i32, name: &str) -> Result<u64, String> {
    let local_addr = local_sym(name)?;

    let self_maps = fs::read_to_string("/proc/self/maps")
        .map_err(|e| format!("self maps: {e}"))?;
    let remote_maps = fs::read_to_string(format!("/proc/{pid}/maps"))
        .map_err(|e| format!("remote maps: {e}"))?;

    for line in self_maps.lines() {
        if !line.contains("r-xp") {
            continue;
        }
        let mut parts = line.split_whitespace();
        let range = parts.next().unwrap_or("");
        let Some((s, e)) = range.split_once('-') else { continue };
        let Ok(start) = u64::from_str_radix(s, 16) else { continue };
        let Ok(end) = u64::from_str_radix(e, 16) else { continue };

        if local_addr >= start && local_addr < end {
            let lib_path = parts.last().unwrap_or("");
            let lib_name = Path::new(lib_path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(lib_path);

            for rline in remote_maps.lines() {
                if rline.contains("r-xp") && rline.contains(lib_name) {
                    let rrange = rline.split_whitespace().next().unwrap_or("");
                    let rstart_str = rrange.split('-').next().unwrap_or("");
                    let remote_base = u64::from_str_radix(rstart_str, 16)
                        .map_err(|e| format!("bad addr: {e}"))?;
                    let offset = local_addr - start;
                    return Ok(remote_base + offset);
                }
            }
            return Err(format!("{lib_name} not found in remote maps"));
        }
    }

    Err(format!("could not locate {name} in any local r-xp region"))
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
pub fn inject_so(pid: i32, so_path: &str) -> Result<(), String> {
    let _guard = ptrace::attach(pid)?;

    let mmap_addr = find_func_remote(pid, "mmap")?;
    eprintln!("  mmap    @ {mmap_addr:#x}");

    let path_c = format!("{so_path}\0");
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

    mem::write_mem(pid, remote_buf, path_c.as_bytes())?;

    let dlopen_addr = find_func_remote(pid, "dlopen")?;
    eprintln!("  dlopen  @ {dlopen_addr:#x}");

    let handle = ptrace::call_remote(pid, dlopen_addr, &[remote_buf, 2])?;
    if handle == 0 {
        return Err("dlopen returned NULL (check .so path is valid in target's mount ns)".into());
    }

    eprintln!("  loaded  handle={handle:#x}");
    Ok(())
}
