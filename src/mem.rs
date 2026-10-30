use std::ffi::c_void;
use std::fs;
use std::io;

#[derive(Clone)]
pub struct Region {
    pub start: u64,
    pub end: u64,
    pub perms: String,
    pub path: String,
}

pub fn read_mem(pid: i32, addr: u64, len: usize) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; len];
    let local = libc::iovec { iov_base: buf.as_mut_ptr() as *mut c_void, iov_len: len };
    let remote = libc::iovec { iov_base: addr as *mut c_void, iov_len: len };
    let n = unsafe { libc::process_vm_readv(pid, &local, 1, &remote, 1, 0) };
    if n < 0 {
        return Err(format!("read {addr:#x}: {}", io::Error::last_os_error()));
    }
    buf.truncate(n as usize);
    Ok(buf)
}

pub fn write_mem(pid: i32, addr: u64, data: &[u8]) -> Result<(), String> {
    let local = libc::iovec { iov_base: data.as_ptr() as *mut c_void, iov_len: data.len() };
    let remote = libc::iovec { iov_base: addr as *mut c_void, iov_len: data.len() };
    let n = unsafe { libc::process_vm_writev(pid, &local, 1, &remote, 1, 0) };
    if n != data.len() as isize {
        return Err(format!("write {addr:#x}: {}", io::Error::last_os_error()));
    }
    Ok(())
}

pub fn maps(pid: i32) -> Result<Vec<Region>, String> {
    let text = fs::read_to_string(format!("/proc/{pid}/maps"))
        .map_err(|e| format!("maps {pid}: {e}"))?;
    let mut out = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(range), Some(perms)) = (it.next(), it.next()) else { continue };
        let Some((s, e)) = range.split_once('-') else { continue };
        let (Ok(start), Ok(end)) = (u64::from_str_radix(s, 16), u64::from_str_radix(e, 16)) else {
            continue;
        };
        let path = it.nth(3).unwrap_or("").to_string();
        out.push(Region { start, end, perms: perms.to_string(), path });
    }
    Ok(out)
}

pub fn find_all(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    let mut hits = Vec::new();
    if needle.is_empty() || hay.len() < needle.len() {
        return hits;
    }
    let last = hay.len() - needle.len();
    let mut i = 0;
    while i <= last {
        if &hay[i..i + needle.len()] == needle {
            hits.push(i);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    hits
}

pub fn scan(pid: i32, needle: &[u8]) -> Result<Vec<u64>, String> {
    let regions = maps(pid)?;
    let mut hits = Vec::new();
    let mut reads_ok = 0u64;
    let mut last_err = String::new();
    for r in regions.iter().filter(|r| r.perms.starts_with("rw")) {
        let mut off = r.start;
        while off < r.end {
            let span = (r.end - off).min(4 << 20) as usize;
            let want = ((r.end - off) as usize).min(span + needle.len().saturating_sub(1));
            match read_mem(pid, off, want) {
                Ok(data) => {
                    reads_ok += 1;
                    for i in find_all(&data, needle) {
                        hits.push(off + i as u64);
                    }
                }
                Err(e) => {
                    last_err = e;
                }
            }
            if (off + span as u64) <= off {
                break;
            }
            off += span as u64;
        }
    }
    if hits.is_empty() && reads_ok == 0 && !last_err.is_empty() {
        return Err(last_err);
    }
    Ok(hits)
}

pub fn hexdump_to_string(data: &[u8], base: u64) -> String {
    let mut out = String::with_capacity(data.len() * 4);
    for (i, chunk) in data.chunks(16).enumerate() {
        use std::fmt::Write;
        let _ = write!(out, "{:016x}  ", base + (i * 16) as u64);
        for b in chunk {
            let _ = write!(out, "{b:02x} ");
        }
        for _ in chunk.len()..16 {
            out.push_str("   ");
        }
        out.push_str(" |");
        for b in chunk {
            out.push(if b.is_ascii_graphic() { *b as char } else { '.' });
        }
        out.push_str("|\n");
    }
    out
}

pub fn hexdump(data: &[u8], base: u64) {
    print!("{}", hexdump_to_string(data, base));
}
