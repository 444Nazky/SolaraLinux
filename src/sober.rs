use std::ffi::CString;
use std::fs;
use std::io;

pub struct NamespaceInfo {
    pub same_pid_ns: bool,
    pub same_mnt_ns: bool,
    pub pid_ns: String,
    pub mnt_ns: String,
    pub flatpak_id: Option<String>,
}

pub fn namespace_info(pid: i32) -> NamespaceInfo {
    let read_ns = |path: &str| {
        fs::read_link(path)
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    };

    let self_pid = read_ns("/proc/self/ns/pid");
    let self_mnt = read_ns("/proc/self/ns/mnt");
    let target_pid = read_ns(&format!("/proc/{pid}/ns/pid"));
    let target_mnt = read_ns(&format!("/proc/{pid}/ns/mnt"));

    let flatpak_id = fs::read(format!("/proc/{pid}/environ"))
        .ok()
        .and_then(|raw| {
            String::from_utf8_lossy(&raw)
                .split('\0')
                .find(|v| v.starts_with("FLATPAK_ID="))
                .map(|v| v.trim_start_matches("FLATPAK_ID=").to_string())
        });

    NamespaceInfo {
        same_pid_ns: self_pid == target_pid,
        same_mnt_ns: self_mnt == target_mnt,
        pid_ns: target_pid,
        mnt_ns: target_mnt,
        flatpak_id,
    }
}

/// Enter a process's mount + PID namespaces, then fork.
/// In the child, exec the given command.
/// Returns in parent after child exits.
pub fn enter_and_exec(pid: i32, args: &[String]) -> Result<(), String> {
    let mnt_path = format!("/proc/{pid}/ns/mnt");
    let pid_path = format!("/proc/{pid}/ns/pid");

    let mnt_fd =
        unsafe { libc::open(CString::new(mnt_path.as_str()).unwrap().as_ptr(), libc::O_RDONLY) };
    if mnt_fd < 0 {
        return Err(format!("open mnt ns: {}", io::Error::last_os_error()));
    }
    let r = unsafe { libc::setns(mnt_fd, libc::CLONE_NEWNS) };
    unsafe { libc::close(mnt_fd) };
    if r != 0 {
        return Err(format!("setns mnt: {}", io::Error::last_os_error()));
    }

    let pid_fd =
        unsafe { libc::open(CString::new(pid_path.as_str()).unwrap().as_ptr(), libc::O_RDONLY) };
    if pid_fd < 0 {
        return Err(format!("open pid ns: {}", io::Error::last_os_error()));
    }
    let r = unsafe { libc::setns(pid_fd, libc::CLONE_NEWPID) };
    unsafe { libc::close(pid_fd) };
    if r != 0 {
        return Err(format!("setns pid: {}", io::Error::last_os_error()));
    }

    let child = unsafe { libc::fork() };
    if child < 0 {
        return Err(format!("fork: {}", io::Error::last_os_error()));
    }

    if child == 0 {
        let cmd = if args.is_empty() { "bash" } else { &args[0] };
        let cstrings: Vec<CString> = if args.is_empty() {
            vec![CString::new("bash").unwrap()]
        } else {
            args.iter()
                .map(|s| CString::new(s.as_str()).unwrap())
                .collect()
        };
        let cmd_c = CString::new(cmd).unwrap();
        let mut argv: Vec<*const libc::c_char> = cstrings.iter().map(|s| s.as_ptr()).collect();
        argv.push(std::ptr::null());
        unsafe { libc::execvp(cmd_c.as_ptr(), argv.as_ptr()) };
        eprintln!("exec failed: {}", io::Error::last_os_error());
        unsafe { libc::_exit(127) };
    }

    let mut status = 0;
    unsafe { libc::waitpid(child, &mut status, 0) };
    Ok(())
}

pub fn find_targets() -> Vec<(i32, String, Option<String>)> {
    let needles = ["sober", "roblox", "wine", "grapejuice", "vinegar"];
    let mut found = Vec::new();

    if let Ok(rd) = fs::read_dir("/proc") {
        for entry in rd.flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else {
                continue;
            };
            // Match on comm + exe basename only. Full cmdline causes false positives
            // from shell wrappers like `bash -c ...roblox...`.
            let comm = fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
            let exe = fs::read_link(format!("/proc/{pid}/exe"))
                .map(|p| {
                    p.file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            let hay = format!("{} {}", comm.trim(), exe).to_lowercase();
            if needles.iter().any(|n| hay.contains(n)) {
                let cmd = fs::read(format!("/proc/{pid}/cmdline"))
                    .map(|raw| String::from_utf8_lossy(&raw).replace('\0', " "))
                    .unwrap_or_default();
                let fpid = namespace_info(pid).flatpak_id;
                found.push((pid, cmd, fpid));
            }
        }
    }
    found
}
