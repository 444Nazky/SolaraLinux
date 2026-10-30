use std::env;
use std::ffi::c_void;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Command, exit};
use std::time::Duration;

const MAGIC: &[u8] = b"SOLARA-SELFTEST-0xDEADBEEF";

#[derive(Clone)]
struct Region {
    start: u64,
    end: u64,
    perms: String,
    path: String,
}

struct Attached {
    pid: i32,
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

fn die(msg: &str) -> ! {
    eprintln!("error: {msg}");
    exit(1)
}

fn attach(pid: i32) -> Attached {
    let r = unsafe {
        libc::ptrace(
            libc::PTRACE_ATTACH,
            pid,
            std::ptr::null_mut::<c_void>(),
            std::ptr::null_mut::<c_void>(),
        )
    };
    if r == -1 {
        die(&format!(
            "ptrace attach on {pid} failed: {}\n\
             hint: same UID required; check /proc/sys/kernel/yama/ptrace_scope (0 = free, 1 = parent only)",
            io::Error::last_os_error()
        ));
    }
    let mut status = 0;
    unsafe { libc::waitpid(pid, &mut status, 0) };
    Attached { pid }
}

fn read_mem(pid: i32, addr: u64, len: usize) -> Result<Vec<u8>, String> {
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

fn write_mem(pid: i32, addr: u64, data: &[u8]) -> Result<(), String> {
    let local = libc::iovec { iov_base: data.as_ptr() as *mut c_void, iov_len: data.len() };
    let remote = libc::iovec { iov_base: addr as *mut c_void, iov_len: data.len() };
    let n = unsafe { libc::process_vm_writev(pid, &local, 1, &remote, 1, 0) };
    if n != data.len() as isize {
        return Err(format!("write {addr:#x}: {}", io::Error::last_os_error()));
    }
    Ok(())
}

fn maps(pid: i32) -> Result<Vec<Region>, String> {
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

fn find_all(hay: &[u8], needle: &[u8]) -> Vec<usize> {
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

fn scan(pid: i32, needle: &[u8]) -> Result<Vec<u64>, String> {
    let regions = maps(pid)?;
    let mut hits = Vec::new();
    for r in regions.iter().filter(|r| r.perms.starts_with("rw")) {
        let mut off = r.start;
        while off < r.end {
            let span = (r.end - off).min(4 << 20) as usize;
            let want = ((r.end - off) as usize).min(span + needle.len().saturating_sub(1));
            if let Ok(data) = read_mem(pid, off, want) {
                for i in find_all(&data, needle) {
                    hits.push(off + i as u64);
                }
            }
            if (off + span as u64) <= off {
                break;
            }
            off += span as u64;
        }
    }
    Ok(hits)
}

fn find_processes() {
    let needles = ["roblox", "sober", "vinegar", "wine", "grapejuice"];
    let mut found = 0;
    let Ok(rd) = fs::read_dir("/proc") else { return };
    for entry in rd.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else {
            continue;
        };
        let Ok(raw) = fs::read(format!("/proc/{pid}/cmdline")) else { continue };
        let cmd = String::from_utf8_lossy(&raw).replace('\0', " ");
        let comm = fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        let hay = format!("{comm} {cmd}").to_lowercase();
        if needles.iter().any(|n| hay.contains(n)) {
            let exe = fs::read_link(format!("/proc/{pid}/exe"))
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "?".into());
            println!("{pid:>8}  {:<28} {exe}", comm.trim());
            found += 1;
        }
    }
    if found == 0 {
        println!("no roblox/wine/sober process found");
    }
}

fn hexdump(data: &[u8], base: u64) {
    for (i, chunk) in data.chunks(16).enumerate() {
        print!("{:016x}  ", base + (i * 16) as u64);
        for b in chunk {
            print!("{b:02x} ");
        }
        for _ in chunk.len()..16 {
            print!("   ");
        }
        print!(" |");
        for b in chunk {
            print!("{}", if b.is_ascii_graphic() { *b as char } else { '.' });
        }
        println!("|");
    }
}

fn parse_hex(s: &str) -> u64 {
    let s = s.trim_start_matches("0x");
    u64::from_str_radix(s, 16).unwrap_or_else(|_| die(&format!("bad hex address: {s}")))
}

fn parse_hex_bytes(s: &str) -> Vec<u8> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace() && *c != '\\').collect();
    let clean = clean.trim_start_matches("0x");
    if clean.len() % 2 != 0 {
        die("hex byte string must have an even number of digits");
    }
    (0..clean.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).unwrap_or_else(|_| die("bad hex byte")))
        .collect()
}

fn selftest() {
    let exe = env::current_exe().unwrap_or_else(|e| die(&format!("current_exe: {e}")));
    let mut child = Command::new(exe)
        .arg("--child")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| die(&format!("spawn child: {e}")));

    let mut reader = BufReader::new(child.stdout.take().expect("child stdout"));
    let mut line = String::new();
    reader.read_line(&mut line).expect("read child addr");
    let addr = parse_hex(line.trim());
    let pid = child.id() as i32;

    let guard = attach(pid);

    let got = read_mem(pid, addr, MAGIC.len()).expect("read child magic");
    assert_eq!(got, MAGIC, "read-back mismatch");
    println!("ok   attach pid={pid} addr={addr:#x}");
    println!("ok   read   {:?}", String::from_utf8_lossy(&got));

    let patch = b"SOLARA-PATCHED-0xCAFEBABE";
    write_mem(pid, addr, patch).expect("write child magic");
    let verify = read_mem(pid, addr, patch.len()).expect("verify write");
    assert_eq!(verify, patch, "write not applied");
    println!("ok   write  {:?}", String::from_utf8_lossy(&verify));

    let hits = scan(pid, b"SOLARA-PATCHED").expect("scan");
    assert!(hits.iter().any(|h| *h == addr), "scan missed patched value");
    println!("ok   scan   {} hit(s), target at {addr:#x}", hits.len());

    drop(guard);
    let _ = child.kill();
    let _ = child.wait();
    println!("PASS  all primitives verified");
}

fn child_mode() {
    let mut buf = vec![0u8; 4096];
    buf[..MAGIC.len()].copy_from_slice(MAGIC);
    println!("{:x}", buf.as_ptr() as usize);
    io::stdout().flush().ok();
    loop {
        std::thread::sleep(Duration::from_secs(1));
        std::hint::black_box(&buf);
    }
}

fn usage() {
    println!(
        "solara - linux roblox process toolkit\n\
         \n\
         usage:\n\
         \x20 solara find                       list roblox/wine/sober processes\n\
         \x20 solara maps <pid> [filter]        memory map of a process\n\
         \x20 solara read <pid> <addr> <len>    hexdump remote memory\n\
         \x20 solara write <pid> <addr> <hex>   write bytes to remote memory\n\
         \x20 solara scan <pid> <hex>           scan writable regions for a byte pattern\n\
         \x20 solara selftest                   verify attach/read/write/scan end-to-end\n\
         \n\
         addresses and byte patterns are hex, optional 0x prefix\n\
         requires same-UID target and ptrace permission\n\
         (/proc/sys/kernel/yama/ptrace_scope = 0, or run as root)"
    );
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let Some(cmd) = args.get(1).map(String::as_str) else {
        usage();
        return;
    };
    let pid = || -> i32 {
        args.get(2)
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or_else(|| die("missing or invalid pid"))
    };

    match cmd {
        "--child" => child_mode(),
        "find" => find_processes(),
        "maps" => {
            let pid = pid();
            let filter = args.get(3).cloned().unwrap_or_default().to_lowercase();
            for r in maps(pid).unwrap_or_else(|e| die(&e)) {
                if filter.is_empty() || r.path.to_lowercase().contains(&filter) {
                    println!(
                        "{:012x}-{:012x} {:<5} {:>10}  {}",
                        r.start,
                        r.end,
                        r.perms,
                        r.end - r.start,
                        r.path
                    );
                }
            }
        }
        "read" => {
            let pid = pid();
            let addr = parse_hex(args.get(3).unwrap_or_else(|| die("missing address")));
            let len: usize = args
                .get(4)
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| die("missing length"));
            let _guard = attach(pid);
            let data = read_mem(pid, addr, len).unwrap_or_else(|e| die(&e));
            hexdump(&data, addr);
        }
        "write" => {
            let pid = pid();
            let addr = parse_hex(args.get(3).unwrap_or_else(|| die("missing address")));
            let bytes = parse_hex_bytes(args.get(4).unwrap_or_else(|| die("missing bytes")));
            let _guard = attach(pid);
            write_mem(pid, addr, &bytes).unwrap_or_else(|e| die(&e));
            println!("wrote {} byte(s) to {addr:#x}", bytes.len());
        }
        "scan" => {
            let pid = pid();
            let needle = parse_hex_bytes(args.get(3).unwrap_or_else(|| die("missing pattern")));
            let _guard = attach(pid);
            let hits = scan(pid, &needle).unwrap_or_else(|e| die(&e));
            for h in &hits {
                println!("{h:#x}");
            }
            eprintln!("{} hit(s)", hits.len());
        }
        "selftest" => selftest(),
        _ => usage(),
    }
}
