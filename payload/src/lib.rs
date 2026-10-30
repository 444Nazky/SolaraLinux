use std::io::{Read, Write};
use std::os::unix::net::UnixListener;

const SOCK: &str = "/tmp/solara-exec.sock";

/// Exported so the injector can call it via dlsym after dlopen.
/// Runs the IPC socket server on a background thread.
#[no_mangle]
pub extern "C" fn solara_payload_start() {
    std::thread::spawn(server);
}

fn server() {
    let _ = std::fs::remove_file(SOCK);
    let l = match UnixListener::bind(SOCK) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[solara-payload] bind {SOCK}: {e}");
            return;
        }
    };
    loop {
        let Ok((mut s, _)) = l.accept() else { continue };
        let mut hdr = [0u8; 4];
        if s.read_exact(&mut hdr).is_err() {
            continue;
        }
        let len = u32::from_le_bytes(hdr) as usize;
        if len > 1 << 20 {
            continue;
        }
        let mut buf = vec![0u8; len];
        if s.read_exact(&mut buf).is_err() {
            continue;
        }
        let script = String::from_utf8_lossy(&buf);
        let resp = format!(
            "[solara-payload] pid={} received {} bytes:\n{}\n--\n\
             note: luau vm hook not connected. this confirms injection + ipc only.\n",
            std::process::id(),
            len,
            script
        );
        let rb = resp.as_bytes();
        let rl = (rb.len() as u32).to_le_bytes();
        let _ = s.write_all(&rl);
        let _ = s.write_all(rb);
    }
}
