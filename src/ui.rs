use crate::{inject, mem, sober, trace};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

const INDEX: &str = include_str!("ui_assets/index.html");
const APP_CSS: &str = include_str!("ui_assets/app.css");
const APP_JS: &str = include_str!("ui_assets/app.js");

pub fn serve(port: u16) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| format!("bind 127.0.0.1:{port}: {e}"))?;
    let url = format!("http://127.0.0.1:{port}");
    println!("solara ui  {url}");
    println!("ctrl-c to stop");
    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    for stream in listener.incoming().flatten() {
        thread::spawn(move || handle(stream));
    }
    Ok(())
}

fn handle(stream: TcpStream) {
    let mut reader = match stream.try_clone() {
        Ok(s) => BufReader::new(s),
        Err(_) => return,
    };
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let mut it = line.split_whitespace();
    let method = it.next().unwrap_or("").to_string();
    let path = it.next().unwrap_or("/").to_string();

    let mut len = 0usize;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).is_err() {
            return;
        }
        let t = h.trim_end();
        if t.is_empty() {
            break;
        }
        if let Some(v) = t.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; len.min(1 << 20)];
    if !body.is_empty() && reader.read_exact(&mut body).is_err() {
        return;
    }
    let body = String::from_utf8_lossy(&body).to_string();

    let form = if method == "POST" { parse_form(&body) } else { Vec::new() };
    let (ctype, out) = route(&method, &path, &form);
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
        out.as_bytes().len(),
        out
    );
    let mut w = stream;
    let _ = w.write_all(resp.as_bytes());
    let _ = w.flush();
}

fn route(method: &str, path: &str, form: &[(String, String)]) -> (&'static str, String) {
    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => {
            ("text/html; charset=utf-8", INDEX.to_string())
        }
        ("GET", "/app.css") => ("text/css; charset=utf-8", APP_CSS.to_string()),
        ("GET", "/app.js") => ("application/javascript; charset=utf-8", APP_JS.to_string()),
        ("GET", "/api/processes") => ("text/plain; charset=utf-8", list_processes()),
        ("POST", "/api/attach") => ("text/plain; charset=utf-8", op_attach(form)),
        ("POST", "/api/exec") => ("text/plain; charset=utf-8", op_exec(form)),
        ("POST", "/api/read") => ("text/plain; charset=utf-8", op_read(form)),
        ("POST", "/api/scan") => ("text/plain; charset=utf-8", op_scan(form)),
        _ => ("text/plain; charset=utf-8", "not found".into()),
    }
}

fn get<'a>(form: &'a [(String, String)], key: &str) -> Option<&'a str> {
    form.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn list_processes() -> String {
    let targets = sober::find_targets();
    if targets.is_empty() {
        return String::new();
    }
    targets
        .iter()
        .map(|(pid, cmd, fp)| {
            let info = sober::namespace_info(*pid);
            format!(
                "{}\t{}\t{}\t{}",
                pid,
                fp.as_deref().unwrap_or("-"),
                if info.same_pid_ns { "shared" } else { "isolated" },
                cmd.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn op_attach(form: &[(String, String)]) -> String {
    let Some(pid) = get(form, "pid").and_then(|s| s.parse::<i32>().ok()) else {
        return "error: missing pid".into();
    };
    let Some(so) = crate::payload_path() else {
        return "error: libpayload.so not found (cargo build --release -p payload)".into();
    };
    trace::take();
    let outcome = match inject::inject_so(pid, &so, None) {
        Ok(()) => format!("ok     attached to pid {pid}"),
        Err(e) => format!("error  {e}"),
    };
    let mut lines = trace::take();
    lines.push(outcome);
    lines.join("\n")
}

fn op_exec(form: &[(String, String)]) -> String {
    let Some(pid) = get(form, "pid").and_then(|s| s.parse::<i32>().ok()) else {
        return "error: missing pid".into();
    };
    let script = get(form, "script").unwrap_or("");
    match crate::send_script(pid, script.as_bytes()) {
        Ok(resp) => resp,
        Err(e) => format!(
            "error: {e}\n(inject the payload first: solara inject {pid} <libpayload.so>)"
        ),
    }
}

fn op_read(form: &[(String, String)]) -> String {
    let Some(pid) = get(form, "pid").and_then(|s| s.parse::<i32>().ok()) else {
        return "error: missing pid".into();
    };
    let addr = match get(form, "addr").and_then(|s| parse_hex(s)) {
        Some(a) => a,
        None => return "error: bad addr (hex)".into(),
    };
    let len: usize = get(form, "len").and_then(|s| s.parse().ok()).unwrap_or(64).min(8192);
    match mem::read_mem(pid, addr, len) {
        Ok(data) => mem::hexdump_to_string(&data, addr),
        Err(e) => format!("error: {e}"),
    }
}

fn op_scan(form: &[(String, String)]) -> String {
    let Some(pid) = get(form, "pid").and_then(|s| s.parse::<i32>().ok()) else {
        return "error: missing pid".into();
    };
    let needle = match get(form, "pattern").and_then(parse_hex_bytes) {
        Some(n) => n,
        None => return "error: bad pattern (hex bytes)".into(),
    };
    match mem::scan(pid, &needle) {
        Ok(hits) => {
            if hits.is_empty() {
                "no hits".into()
            } else {
                format!("{} hit(s)\n{}", hits.len(), hits.iter().map(|h| format!("{h:#x}")).collect::<Vec<_>>().join("\n"))
            }
        }
        Err(e) => format!("error: {e}"),
    }
}

fn parse_form(body: &str) -> Vec<(String, String)> {
    body.split('&')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            Some((urldecode(k), urldecode(v)))
        })
        .collect()
}

fn urldecode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b[i]);
                        i += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_hex(s: &str) -> Option<u64> {
    let s = s.trim().trim_start_matches("0x");
    u64::from_str_radix(s, 16).ok()
}

fn parse_hex_bytes(s: &str) -> Option<Vec<u8>> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace() && *c != '\\').collect();
    let clean = clean.trim_start_matches("0x");
    if clean.is_empty() || clean.len() % 2 != 0 {
        return None;
    }
    (0..clean.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).ok())
        .collect()
}
