use std::io::{Read, Write};
use std::os::unix::net::UnixListener;

const SOCK: &str = "/tmp/solara-exec.sock";

extern "C" fn _solara_init() {
    std::thread::spawn(server);
}

#[link_section = ".init_array"]
static INIT: [extern "C" fn(); 1] = [_solara_init];

fn server() {
    let _ = std::fs::remove_file(SOCK);
    let l = match UnixListener::bind(SOCK) {
        Ok(l) => l,
        Err(_) => return,
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
