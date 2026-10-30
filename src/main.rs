mod inject;
mod mem;
mod ptrace;
mod sober;

use std::env;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, exit};
use std::time::Duration;

const MAGIC: &[u8] = b"SOLARA-SELFTEST-0xDEADBEEF";
const SOCK: &str = "/tmp/solara-exec.sock";

fn die(msg: &str) -> ! {
    eprintln!("error: {msg}");
    exit(1)
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

fn find_processes() {
    let targets = sober::find_targets();
    if targets.is_empty() {
        println!("no roblox/wine/sober process found");
        return;
    }
    println!("{:>8}  {:<10} {:<20} {}", "pid", "flatpak", "ns", "cmdline");
    for (pid, cmd, fp) in targets {
        let info = sober::namespace_info(pid);
        let ns = if info.same_pid_ns { "shared" } else { "isolated" };
        let fp = fp.unwrap_or_else(|| "-".into());
        let short = if cmd.len() > 60 { &cmd[..60] } else { &cmd };
        println!("{pid:>8}  {fp:<20} {ns:<10} {short}");
    }
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

    let guard = ptrace::attach(pid).unwrap_or_else(|e| die(&e));

    let got = mem::read_mem(pid, addr, MAGIC.len()).expect("read child magic");
    assert_eq!(got, MAGIC, "read-back mismatch");
    println!("ok   attach pid={pid} addr={addr:#x}");
    println!("ok   read   {:?}", String::from_utf8_lossy(&got));

    let patch = b"SOLARA-PATCHED-0xCAFEBABE";
    mem::write_mem(pid, addr, patch).expect("write child magic");
    let verify = mem::read_mem(pid, addr, patch.len()).expect("verify write");
    assert_eq!(verify, patch, "write not applied");
    println!("ok   write  {:?}", String::from_utf8_lossy(&verify));

    let hits = mem::scan(pid, b"SOLARA-PATCHED").expect("scan");
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

fn payload_path() -> Option<String> {
    let exe = env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidates = [
        dir.join("libpayload.so"),
        dir.join("../payload/libpayload.so"),
        dir.join("../../payload/libpayload.so"),
        dir.join("../../target/release/libpayload.so"),
    ];
    candidates
        .iter()
        .find(|p| p.exists())
        .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()).display().to_string())
}

fn inject_test() {
    let so = payload_path().unwrap_or_else(|| {
        die("libpayload.so not found. build it: cargo build --release -p payload")
    });
    println!("payload: {so}");

    let mut child = Command::new("sleep")
        .arg("1000")
        .spawn()
        .unwrap_or_else(|e| die(&format!("spawn target: {e}")));
    let pid = child.id() as i32;
    println!("target:  pid={pid}");

    let result = inject::inject_so(pid, &so);
    if let Err(e) = result {
        let _ = child.kill();
        die(&format!("inject failed: {e}"));
    }

    let mut ready = false;
    for _ in 0..50 {
        if Path::new(SOCK).exists() {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !ready {
        let _ = child.kill();
        die("payload socket never appeared (constructor did not run?)");
    }
    println!("socket:  {SOCK}");

    let script = b"print('hello from solara')";
    match send_script(script) {
        Ok(resp) => {
            println!("resp:    {}", resp.replace('\n', "\n         "));
            println!("PASS  injection + ipc verified end-to-end");
        }
        Err(e) => {
            let _ = child.kill();
            die(&format!("ipc failed: {e}"));
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(SOCK);
}

fn send_script(script: &[u8]) -> Result<String, String> {
    let mut s = UnixStream::connect(SOCK).map_err(|e| format!("connect {SOCK}: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    s.write_all(&(script.len() as u32).to_le_bytes())
        .map_err(|e| e.to_string())?;
    s.write_all(script).map_err(|e| e.to_string())?;

    let mut hdr = [0u8; 4];
    s.read_exact(&mut hdr).map_err(|e| e.to_string())?;
    let len = u32::from_le_bytes(hdr) as usize;
    let mut buf = vec![0u8; len];
    s.read_exact(&mut buf).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn read_script_arg(arg: &str) -> Vec<u8> {
    let p = Path::new(arg);
    if p.is_file() {
        std::fs::read(p).unwrap_or_else(|e| die(&format!("read {arg}: {e}")))
    } else {
        arg.as_bytes().to_vec()
    }
}

fn usage() {
    println!(
        "solara - linux roblox process toolkit\n\
         \n\
         recon:\n\
         \x20 solara find                       list roblox/wine/sober processes + ns status\n\
         \x20 solara maps <pid> [filter]        memory map of a process\n\
         \x20 solara scan <pid> <hex>           scan writable regions for a byte pattern\n\
         \n\
         memory:\n\
         \x20 solara read <pid> <addr> <len>    hexdump remote memory\n\
         \x20 solara write <pid> <addr> <hex>   write bytes to remote memory\n\
         \n\
         sober:\n\
         \x20 solara sober info <pid>           namespace + flatpak info\n\
         \x20 solara sober enter <pid> [cmd..]  enter target ns (shell by default)\n\
         \n\
         execute:\n\
         \x20 solara inject <pid> <path.so>     ptrace+dlopen inject a shared object\n\
         \x20 solara exec <pid> <script|file>   send script to injected payload\n\
         \x20 solara inject-test                end-to-end inject+ipc self-test\n\
         \n\
         verify:\n\
         \x20 solara selftest                   verify attach/read/write/scan\n\
         \n\
         addresses/patterns are hex, 0x prefix optional\n\
         needs same-UID target; ptrace_scope=0 or root for non-child processes\n\
         sober runs in a flatpak sandbox: use 'solara sober enter' first, or run root"
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
            for r in mem::maps(pid).unwrap_or_else(|e| die(&e)) {
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
            let _guard = ptrace::attach(pid).unwrap_or_else(|e| die(&e));
            let data = mem::read_mem(pid, addr, len).unwrap_or_else(|e| die(&e));
            mem::hexdump(&data, addr);
        }
        "write" => {
            let pid = pid();
            let addr = parse_hex(args.get(3).unwrap_or_else(|| die("missing address")));
            let bytes = parse_hex_bytes(args.get(4).unwrap_or_else(|| die("missing bytes")));
            let _guard = ptrace::attach(pid).unwrap_or_else(|e| die(&e));
            mem::write_mem(pid, addr, &bytes).unwrap_or_else(|e| die(&e));
            println!("wrote {} byte(s) to {addr:#x}", bytes.len());
        }
        "scan" => {
            let pid = pid();
            let needle = parse_hex_bytes(args.get(3).unwrap_or_else(|| die("missing pattern")));
            let _guard = ptrace::attach(pid).unwrap_or_else(|e| die(&e));
            let hits = mem::scan(pid, &needle).unwrap_or_else(|e| die(&e));
            for h in &hits {
                println!("{h:#x}");
            }
            eprintln!("{} hit(s)", hits.len());
        }
        "sober" => {
            let sub = args.get(2).map(String::as_str).unwrap_or("find");
            match sub {
                "find" => find_processes(),
                "info" => {
                    let pid = args
                        .get(3)
                        .and_then(|s| s.parse::<i32>().ok())
                        .unwrap_or_else(|| die("usage: solara sober info <pid>"));
                    let i = sober::namespace_info(pid);
                    println!("pid_ns:    {}", i.pid_ns);
                    println!("mnt_ns:    {}", i.mnt_ns);
                    println!("same_pid:  {}", i.same_pid_ns);
                    println!("same_mnt:  {}", i.same_mnt_ns);
                    println!("flatpak:   {}", i.flatpak_id.as_deref().unwrap_or("-"));
                }
                "enter" => {
                    let pid = args
                        .get(3)
                        .and_then(|s| s.parse::<i32>().ok())
                        .unwrap_or_else(|| die("usage: solara sober enter <pid> [cmd..]"));
                    let rest: Vec<String> = args[4..].to_vec();
                    if let Err(e) = sober::enter_and_exec(pid, &rest) {
                        die(&e);
                    }
                }
                _ => usage(),
            }
        }
        "inject" => {
            let pid = pid();
            let so = args.get(3).unwrap_or_else(|| die("usage: solara inject <pid> <path.so>"));
            inject::inject_so(pid, so).unwrap_or_else(|e| die(&e));
            println!("injected {so} into pid {pid}");
        }
        "exec" => {
            let pid = pid();
            let script = args
                .get(3)
                .unwrap_or_else(|| die("usage: solara exec <pid> <script|file>"));
            let bytes = read_script_arg(script);
            match send_script(&bytes) {
                Ok(resp) => println!("{resp}"),
                Err(e) => die(&format!("{e}\n(inject the payload first: solara inject {pid} <libpayload.so>)")),
            }
        }
        "inject-test" => inject_test(),
        "selftest" => selftest(),
        _ => usage(),
    }
}
