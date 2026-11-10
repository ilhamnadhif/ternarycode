//! Process information for the foreground job of a pane: its arguments, executable and working
//! directory. macOS reads the kernel (`sysctl KERN_PROCARGS2`, `proc_pidinfo`); Linux reads
//! `/proc`; Windows asks for the process's image name and command line (its working directory
//! is not readable from outside: the shell reports it with OSC 7).

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: i32,
    pub argv: Vec<String>,
    pub exe: Option<String>,
}

impl ProcInfo {
    /// A short name for display: the program's file name (`-zsh` → `zsh`, `C:\…\node.exe` →
    /// `node`).
    pub fn name(&self) -> String {
        let first = self
            .argv
            .first()
            .map(|s| s.as_str())
            .or(self.exe.as_deref())
            .unwrap_or("?");
        let name = first
            .trim_start_matches('-')
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(first);
        match name.len().checked_sub(4) {
            Some(i) if name.is_char_boundary(i) && name[i..].eq_ignore_ascii_case(".exe") => {
                name[..i].to_string()
            }
            _ => name.to_string(),
        }
    }
}

#[cfg(target_os = "macos")]
pub fn info(pid: i32) -> Option<ProcInfo> {
    // Size of the argument area, then the area: argc, the executable path, padding, argv, env.
    let mut argmax: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    // SAFETY: sysctl with correctly sized buffers.
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&mut argmax as *mut libc::c_int).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    let mut buf = vec![0u8; argmax.max(4096) as usize];
    let mut size = buf.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    // SAFETY: as above.
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    buf.truncate(size);
    parse_procargs2(pid, &buf)
}

/// Parses the `KERN_PROCARGS2` layout.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_procargs2(pid: i32, buf: &[u8]) -> Option<ProcInfo> {
    if buf.len() < 4 {
        return None;
    }
    let argc = i32::from_ne_bytes(buf[..4].try_into().ok()?).max(0) as usize;
    let rest = &buf[4..];
    let end = rest.iter().position(|b| *b == 0)?;
    let exe = String::from_utf8_lossy(&rest[..end]).into_owned();
    let mut i = end;
    while i < rest.len() && rest[i] == 0 {
        i += 1;
    }
    let mut argv = Vec::with_capacity(argc);
    while argv.len() < argc && i < rest.len() {
        let e = rest[i..].iter().position(|b| *b == 0).map_or(rest.len(), |p| i + p);
        argv.push(String::from_utf8_lossy(&rest[i..e]).into_owned());
        i = e + 1;
    }
    Some(ProcInfo {
        pid,
        argv,
        exe: Some(exe).filter(|e| !e.is_empty()),
    })
}

#[cfg(target_os = "macos")]
pub fn cwd(pid: i32) -> Option<PathBuf> {
    // SAFETY: proc_vnodepathinfo is plain old data; proc_pidinfo fills it.
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
    let r = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            (&mut info as *mut libc::proc_vnodepathinfo).cast(),
            size,
        )
    };
    if r != size {
        return None;
    }
    // vip_path is [[c_char; 32]; 32], i.e. MAXPATHLEN bytes.
    let raw: &[u8] = unsafe {
        std::slice::from_raw_parts(info.pvi_cdir.vip_path.as_ptr().cast::<u8>(), 32 * 32)
    };
    let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
    (end > 0).then(|| PathBuf::from(String::from_utf8_lossy(&raw[..end]).into_owned()))
}

/// The parent process id.
#[cfg(target_os = "macos")]
pub fn ppid(pid: i32) -> Option<i32> {
    // SAFETY: proc_bsdinfo is plain old data; proc_pidinfo fills it.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let r = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    (r == size).then_some(info.pbi_ppid as i32)
}

#[cfg(target_os = "linux")]
pub fn ppid(pid: i32) -> Option<i32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // pid (comm) state ppid …; comm may contain spaces and parentheses.
    let after = &stat[stat.rfind(')')? + 2..];
    after.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn ppid(_pid: i32) -> Option<i32> {
    None
}

/// Resident memory of this process in bytes (for the status bar).
#[cfg(target_os = "macos")]
pub fn own_resident_bytes() -> Option<u64> {
    // SAFETY: proc_taskinfo is plain old data; proc_pidinfo fills it.
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    let r = unsafe {
        libc::proc_pidinfo(
            std::process::id() as i32,
            libc::PROC_PIDTASKINFO,
            0,
            (&mut info as *mut libc::proc_taskinfo).cast(),
            size,
        )
    };
    (r == size).then_some(info.pti_resident_size)
}

#[cfg(target_os = "linux")]
pub fn own_resident_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    // SAFETY: sysconf is always safe to call.
    Some(pages * unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn own_resident_bytes() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
pub fn info(pid: i32) -> Option<ProcInfo> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let argv = raw
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    let exe = std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    Some(ProcInfo { pid, argv, exe })
}

#[cfg(target_os = "linux")]
pub fn cwd(pid: i32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn info(_pid: i32) -> Option<ProcInfo> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn cwd(_pid: i32) -> Option<PathBuf> {
    None
}

#[cfg(windows)]
pub use win::{info, own_resident_bytes, ppid};

#[cfg(windows)]
mod win {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

    use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessCommandLineInformation};
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, UNICODE_STRING};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };

    use super::{ProcInfo, split_command_line};

    fn open(pid: i32) -> Option<OwnedHandle> {
        let pid = u32::try_from(pid).ok()?;
        // SAFETY: OpenProcess returns null on failure; the handle is owned from here on.
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        (!h.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(h) })
    }

    fn image_name(h: HANDLE) -> Option<String> {
        let mut buf = vec![0u16; 32 * 1024];
        let mut len = buf.len() as u32;
        // SAFETY: `len` is the buffer's size in characters; it gets the length written.
        let ok = unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) };
        (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }

    /// The command line the process was started with (Windows 8.1 and later).
    fn command_line(h: HANDLE) -> Option<String> {
        const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;
        let mut size = 4096u32;
        for _ in 0..3 {
            // u64 storage: the UNICODE_STRING at its start is pointer-aligned.
            let mut buf = vec![0u64; (size as usize).div_ceil(8)];
            let mut needed = 0u32;
            // SAFETY: `buf` has `size` bytes; the call fills a UNICODE_STRING followed by its text.
            let status = unsafe {
                NtQueryInformationProcess(
                    h,
                    ProcessCommandLineInformation,
                    buf.as_mut_ptr().cast(),
                    size,
                    &mut needed,
                )
            };
            if status == STATUS_INFO_LENGTH_MISMATCH && needed > size {
                size = needed;
                continue;
            }
            if status < 0 {
                return None;
            }
            // SAFETY: on success the buffer starts with a UNICODE_STRING whose text lies in it.
            let text = unsafe {
                let us = &*(buf.as_ptr() as *const UNICODE_STRING);
                if us.Buffer.is_null() {
                    return None;
                }
                std::slice::from_raw_parts(us.Buffer, us.Length as usize / 2)
            };
            return Some(String::from_utf16_lossy(text));
        }
        None
    }

    pub fn info(pid: i32) -> Option<ProcInfo> {
        let h = open(pid)?;
        let raw = h.as_raw_handle() as HANDLE;
        let exe = image_name(raw);
        let mut argv = command_line(raw).map(|c| split_command_line(&c)).unwrap_or_default();
        if argv.is_empty() {
            argv.extend(exe.clone());
        }
        if argv.is_empty() {
            return None;
        }
        Some(ProcInfo { pid, argv, exe })
    }

    pub fn ppid(pid: i32) -> Option<i32> {
        let pid = u32::try_from(pid).ok()?;
        // SAFETY: a snapshot handle, owned; entries are plain data.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap.is_null() || snap == INVALID_HANDLE_VALUE {
                return None;
            }
            let snapshot = OwnedHandle::from_raw_handle(snap);
            let raw = snapshot.as_raw_handle() as HANDLE;
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            if Process32FirstW(raw, &mut entry) == 0 {
                return None;
            }
            loop {
                if entry.th32ProcessID == pid {
                    return i32::try_from(entry.th32ParentProcessID).ok();
                }
                if Process32NextW(raw, &mut entry) == 0 {
                    return None;
                }
            }
        }
    }

    /// The working set of this process (what Task Manager counts as its memory in use).
    pub fn own_resident_bytes() -> Option<u64> {
        // SAFETY: plain data filled by the call; the pseudo handle needs no closing.
        unsafe {
            let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            let cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            (K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, cb) != 0)
                .then_some(pmc.WorkingSetSize as u64)
        }
    }
}

/// Splits a Windows command line into arguments the way the C runtime does (quotes group,
/// backslashes escape a quote).
#[cfg_attr(not(windows), allow(dead_code))]
fn split_command_line(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let (mut in_arg, mut quoted, mut backslashes) = (false, false, 0usize);
    for c in line.chars() {
        match c {
            '\\' => {
                backslashes += 1;
                in_arg = true;
                continue;
            }
            '"' => {
                cur.extend(std::iter::repeat_n('\\', backslashes / 2));
                if backslashes % 2 == 1 {
                    cur.push('"');
                } else {
                    quoted = !quoted;
                }
                backslashes = 0;
                in_arg = true;
                continue;
            }
            _ => {
                cur.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
            }
        }
        if (c == ' ' || c == '\t') && !quoted {
            if in_arg {
                args.push(std::mem::take(&mut cur));
                in_arg = false;
            }
        } else {
            cur.push(c);
            in_arg = true;
        }
    }
    cur.extend(std::iter::repeat_n('\\', backslashes));
    if in_arg {
        args.push(cur);
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_procargs2_layout() {
        let mut buf = 3i32.to_ne_bytes().to_vec();
        buf.extend_from_slice(b"/usr/local/bin/node\0\0\0\0");
        buf.extend_from_slice(b"node\0/opt/x/cli.js\0--continue\0HOME=/Users/me\0");
        let p = parse_procargs2(7, &buf).unwrap();
        assert_eq!(p.argv, ["node", "/opt/x/cli.js", "--continue"]);
        assert_eq!(p.exe.as_deref(), Some("/usr/local/bin/node"));
        assert_eq!(p.name(), "node");
    }

    #[test]
    fn splits_windows_command_lines_and_names_programs() {
        let argv = split_command_line(
            r#""C:\Program Files\nodejs\node.exe" C:\Users\me\npm\node_modules\@anthropic-ai\claude-code\cli.js --resume "a \"b\" c" x\\"#,
        );
        assert_eq!(
            argv,
            [
                r"C:\Program Files\nodejs\node.exe",
                r"C:\Users\me\npm\node_modules\@anthropic-ai\claude-code\cli.js",
                "--resume",
                r#"a "b" c"#,
                r"x\\",
            ]
        );
        assert_eq!(split_command_line(r#"a "" b"#), ["a", "", "b"]);
        let p = ProcInfo { pid: 1, argv: argv.clone(), exe: None };
        assert_eq!(p.name(), "node");
        assert_eq!(
            tc_core::agent::recognize(&argv, Some(r"C:\Program Files\nodejs\node.exe")),
            Some(tc_core::agent::AgentKind::Claude)
        );
    }

    #[cfg(windows)]
    #[test]
    fn reads_a_live_process() {
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/c", "ping -n 5 127.0.0.1 >nul"])
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let pid = child.id() as i32;
        let p = info(pid).expect("info");
        assert_eq!(p.name().to_ascii_lowercase(), "cmd");
        assert!(p.argv.iter().any(|a| a.contains("ping")), "{:?}", p.argv);
        assert!(p.exe.as_deref().is_some_and(|e| e.to_ascii_lowercase().ends_with("cmd.exe")));
        assert_eq!(ppid(pid), Some(std::process::id() as i32));
        assert!(own_resident_bytes().unwrap() > 1024 * 1024);
        let _ = child.kill();
        let _ = child.wait();
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn reads_a_live_process() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("5")
            .current_dir("/tmp")
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        let pid = child.id() as i32;
        let p = info(pid).expect("info");
        assert_eq!(p.argv, ["/bin/sleep", "5"]);
        assert_eq!(p.name(), "sleep");
        let dir = cwd(pid).expect("cwd");
        assert_eq!(dir.canonicalize().unwrap(), PathBuf::from("/tmp").canonicalize().unwrap());
        assert_eq!(ppid(pid), Some(std::process::id() as i32));
        assert!(own_resident_bytes().unwrap() > 1024 * 1024);
        let _ = child.kill();
        let _ = child.wait();
    }
}
