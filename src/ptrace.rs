use crate::mem;
use std::ffi::c_void;
use std::io;

pub struct Attached {
    pub pid: i32,
}

impl Drop for Attached {
    fn drop(&mut self) {
        unsafe {
            libc::ptrace(
                libc::PTRACE_DETACH,
                self.pid,
                std::ptr::null_mut::<c_void>(),
                std::ptr::null_mut::<c_void>(),
            );
        }
    }
}

pub fn attach(pid: i32) -> Result<Attached, String> {
    let r = unsafe {
        libc::ptrace(
            libc::PTRACE_ATTACH,
            pid,
            std::ptr::null_mut::<c_void>(),
            std::ptr::null_mut::<c_void>(),
        )
    };
    if r == -1 {
        return Err(format!(
            "ptrace attach pid={pid}: {}\n\
             hint: check /proc/sys/kernel/yama/ptrace_scope (0=free, 1=parent-only)\n\
             hint: for sober flatpak: run inside the sandbox or as root",
            io::Error::last_os_error()
        ));
    }
    let mut status = 0;
    unsafe { libc::waitpid(pid, &mut status, 0) };
    Ok(Attached { pid })
}

pub fn get_regs(pid: i32) -> Result<libc::user_regs_struct, String> {
    let mut regs: libc::user_regs_struct = unsafe { std::mem::zeroed() };
    let r = unsafe {
        libc::ptrace(
            libc::PTRACE_GETREGS,
            pid,
            std::ptr::null_mut::<c_void>(),
            &mut regs as *mut _ as *mut c_void,
        )
    };
    if r == -1 {
        Err(format!("PTRACE_GETREGS: {}", io::Error::last_os_error()))
    } else {
        Ok(regs)
    }
}

pub fn set_regs(pid: i32, regs: &libc::user_regs_struct) -> Result<(), String> {
    let r = unsafe {
        libc::ptrace(
            libc::PTRACE_SETREGS,
            pid,
            std::ptr::null_mut::<c_void>(),
            regs as *const _ as *mut c_void,
        )
    };
    if r == -1 {
        Err(format!("PTRACE_SETREGS: {}", io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

pub fn cont(pid: i32) -> Result<(), String> {
    let r = unsafe {
        libc::ptrace(
            libc::PTRACE_CONT,
            pid,
            std::ptr::null_mut::<c_void>(),
            std::ptr::null_mut::<c_void>(),
        )
    };
    if r == -1 {
        Err(format!("PTRACE_CONT: {}", io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

fn poke_word(pid: i32, addr: u64, word: i64) -> Result<(), String> {
    let r = unsafe {
        libc::ptrace(
            libc::PTRACE_POKETEXT,
            pid,
            addr as usize as *mut c_void,
            word as usize as *mut c_void,
        )
    };
    if r == -1 {
        Err(format!("POKETEXT {addr:#x}: {}", io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

fn read_code_byte(pid: i32, addr: u64) -> Result<u8, String> {
    Ok(mem::read_mem(pid, addr, 1)?[0])
}

fn write_code_byte(pid: i32, addr: u64, byte: u8) -> Result<(), String> {
    let buf = mem::read_mem(pid, addr, 8)
        .map_err(|e| format!("read word at {addr:#x}: {e}"))?;
    if buf.len() < 8 {
        return Err(format!("short read at {addr:#x}: {} bytes", buf.len()));
    }
    let mut word = i64::from_le_bytes(buf[0..8].try_into().unwrap());
    word = (word & !0xFFi64) | (byte as i64);
    poke_word(pid, addr, word)
}

/// Call a function in the target process by:
/// 1. Save regs, save original byte at return address
/// 2. Write 0xCC (int3) at return address (PTRACE_POKETEXT bypasses r-x page protection)
/// 3. Set rip = function, set arg registers (SysV x86_64 ABI)
/// 4. PTRACE_CONT → function executes → returns to 0xCC → SIGTRAP
/// 5. Read rax (return value)
/// 6. Restore original byte + registers
pub fn call_remote(pid: i32, func: u64, args: &[u64]) -> Result<u64, String> {
    if args.len() > 6 {
        return Err("max 6 args".into());
    }

    let saved = get_regs(pid)?;
    let orig_rip = saved.rip;
    let orig_byte = read_code_byte(pid, orig_rip)?;

    write_code_byte(pid, orig_rip, 0xCC)?;

    let mut regs = saved;
    regs.rip = func;
    regs.orig_rax = !0u64;

    let arg_slots: [&mut u64; 6] = [
        &mut regs.rdi, &mut regs.rsi, &mut regs.rdx,
        &mut regs.rcx, &mut regs.r8, &mut regs.r9,
    ];
    for (i, val) in args.iter().enumerate() {
        *arg_slots[i] = *val;
    }

    set_regs(pid, &regs)?;
    cont(pid)?;

    let mut status = 0;
    unsafe { libc::waitpid(pid, &mut status, 0) };

    let result = if libc::WIFEXITED(status) {
        Err("target exited during call".into())
    } else if libc::WIFSTOPPED(status) && libc::WSTOPSIG(status) == libc::SIGTRAP {
        let ret_regs = get_regs(pid)?;
        Ok(ret_regs.rax)
    } else {
        let sig = if libc::WIFSTOPPED(status) { libc::WSTOPSIG(status) } else { -1 };
        Err(format!("expected SIGTRAP, got signal {sig} (status={status:#x})"))
    };

    let _ = write_code_byte(pid, orig_rip, orig_byte);
    let _ = set_regs(pid, &saved);

    result
}
