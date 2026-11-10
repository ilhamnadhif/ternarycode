//! Shell integration (PRD section 8): OSC 133 prompt marks (A prompt start, B prompt end,
//! C command start, D;status command end) and the working directory (OSC 7), injected into zsh,
//! bash and fish through the environment alone. The user's dotfiles are not touched and load as
//! they always do; the scripts (in `shell/`, written to `<data_dir>/shell` by [`install`]) never
//! print anything else, do nothing in non-interactive shells and hand `$?` on unchanged.
//!
//! - zsh: `ZDOTDIR` points at our `zsh/` directory and the user's own `ZDOTDIR`, if any, is kept
//!   in `TERNARYCODE_ZSH_ZDOTDIR`. Our `.zshenv` (the first file zsh reads) restores `ZDOTDIR`
//!   and sources the user's `.zshenv`; zsh then reads `.zprofile`, `.zshrc` and `.zlogin` from
//!   the user's directory itself. The hooks install themselves at the first prompt.
//! - bash: `PROMPT_COMMAND` starts with `. '<dir>/bash/ternarycode.bash';builtin eval "${__tc_late-}"`.
//!   bash reads its startup files itself; at the first prompt the script replaces itself with its
//!   hooks (a DEBUG trap writes C; the `eval` sets it, since bash restores the DEBUG trap it had
//!   when a sourced file returns). bash 3.2, the one macOS ships, ignores `ENV` even in posix mode, so the
//!   `--posix` + `ENV` route is closed without changing the command line.
//! - fish: our directory goes first in `XDG_DATA_DIRS` (`TERNARYCODE_FISH_XDG_DIR`, plus
//!   `TERNARYCODE_FISH_XDG_UNSET` when it was unset), so fish loads
//!   `fish/vendor_conf.d/ternarycode.fish`, which takes it out again. fish 4.0 and later mark
//!   prompts themselves; the script then adds nothing.
//!
//! - PowerShell (Windows): it reads no startup file it could be pointed at through the
//!   environment, so the pane's shell gets `-NoExit -EncodedCommand <script>` instead
//!   ([`windows_shell_args`]), which runs after the user's profile. A command, unlike a script
//!   file, runs under every execution policy. The environment only says it is wanted
//!   ([`ENV_PWSH`]).
//!
//! Only for interactive shells: a pane's shell, not `sh -c` jobs.

use std::fs;
use std::io;
use std::path::Path;

/// The scripts, by path below the install directory.
const FILES: &[(&str, &str)] = &[
    ("zsh/.zshenv", include_str!("../shell/zsh/.zshenv")),
    (
        "zsh/ternarycode-integration.zsh",
        include_str!("../shell/zsh/ternarycode-integration.zsh"),
    ),
    (
        "bash/ternarycode.bash",
        include_str!("../shell/bash/ternarycode.bash"),
    ),
    (
        "fish/vendor_conf.d/ternarycode.fish",
        include_str!("../shell/fish/vendor_conf.d/ternarycode.fish"),
    ),
];

/// The PowerShell integration, run with `-EncodedCommand`.
const POWERSHELL: &str = include_str!("../shell/pwsh/ternarycode.ps1");

/// Set in a pane's environment when its PowerShell should load the integration.
pub const ENV_PWSH: &str = "TERNARYCODE_PWSH_INTEGRATION";

/// Where fish looks when `XDG_DATA_DIRS` is unset.
const XDG_DATA_DIRS_DEFAULT: &str = "/usr/local/share:/usr/share";

/// Writes the scripts into `dir` (TernaryCode passes `<data_dir>/shell`). Files that are already
/// up to date are left alone; the others are replaced atomically, so a shell starting meanwhile
/// never reads half a file.
pub fn install(dir: &Path) -> io::Result<()> {
    for (rel, content) in FILES {
        let path = dir.join(rel);
        if fs::read(&path).is_ok_and(|old| old == content.as_bytes()) {
            continue;
        }
        let parent = path.parent().expect("script paths have a parent");
        fs::create_dir_all(parent)?;
        let name = path.file_name().expect("script paths have a name");
        let tmp = parent.join(format!(
            ".{}.{}.tmp",
            name.to_string_lossy(),
            std::process::id()
        ));
        fs::write(&tmp, content)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644))?;
        }
        if let Err(e) = fs::rename(&tmp, &path) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Zsh,
    Bash,
    Fish,
    PowerShell,
}

/// The shell named by `shell`: a path or a name, also in the `-zsh` form of a login shell's
/// `argv[0]`, and Windows paths (`C:\…\pwsh.exe`).
fn kind(shell: &str) -> Option<Kind> {
    let name = shell.rsplit(['/', '\\']).next()?.trim_start_matches('-');
    let name = match name.len().checked_sub(4) {
        Some(i) if name.is_char_boundary(i) && name[i..].eq_ignore_ascii_case(".exe") => &name[..i],
        _ => name,
    };
    match name.to_ascii_lowercase().as_str() {
        "zsh" => Some(Kind::Zsh),
        "bash" => Some(Kind::Bash),
        "fish" => Some(Kind::Fish),
        "pwsh" | "powershell" => Some(Kind::PowerShell),
        _ => None,
    }
}

/// The arguments for a pane's shell on Windows: PowerShell starts without its banner, and with
/// the integration when `integrated`; other shells get none.
pub fn windows_shell_args(shell: &str, integrated: bool) -> Vec<String> {
    if kind(shell) != Some(Kind::PowerShell) {
        return Vec::new();
    }
    let mut args = vec!["-NoLogo".to_string()];
    if integrated {
        let utf16: Vec<u8> = POWERSHELL
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        args.extend(["-NoExit".into(), "-EncodedCommand".into(), base64(&utf16)]);
    }
    args
}

/// Standard base64 (what `-EncodedCommand` takes, over UTF-16LE text).
fn base64(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The variables to add to a pane's environment so that its shell `shell` (a path or a name)
/// loads the integration installed in `dir`. `current_env` reads the environment the pane would
/// otherwise get. Empty for other shells, and when `dir` cannot be expressed for the shell.
pub fn env(
    dir: &Path,
    shell: &str,
    current_env: impl Fn(&str) -> Option<String>,
) -> Vec<(String, String)> {
    let Some(kind) = kind(shell) else {
        return Vec::new();
    };
    let var = |k: &str, v: String| (k.to_string(), v);
    match kind {
        Kind::PowerShell => vec![var(ENV_PWSH, "1".into())],
        Kind::Zsh => {
            let Some(ours) = dir.join("zsh").to_str().map(str::to_string) else {
                return Vec::new();
            };
            let mut vars = Vec::new();
            // Not our own directory: that would be a pane started from a broken pane.
            if let Some(user) = current_env("ZDOTDIR").filter(|z| Path::new(z) != Path::new(&ours))
            {
                vars.push(var("TERNARYCODE_ZSH_ZDOTDIR", user));
            }
            vars.push(var("ZDOTDIR", ours));
            vars
        }
        Kind::Bash => {
            // The script finds its own command in PROMPT_COMMAND by this exact text.
            let Some(script) = dir
                .join("bash/ternarycode.bash")
                .to_str()
                .map(str::to_string)
            else {
                return Vec::new();
            };
            if script.contains(['\'', '\n']) {
                return Vec::new();
            }
            let boot = format!(". '{script}';builtin eval \"${{__tc_late-}}\"");
            let pc = match current_env("PROMPT_COMMAND") {
                Some(pc) if pc.contains(&boot) => pc,
                Some(pc) if !pc.trim().is_empty() => format!("{boot}\n{pc}"),
                _ => boot,
            };
            vec![var("PROMPT_COMMAND", pc)]
        }
        Kind::Fish => {
            let Some(ours) = dir.to_str().map(str::to_string) else {
                return Vec::new();
            };
            if ours.is_empty() || ours.contains(':') {
                return Vec::new();
            }
            let mut vars = Vec::new();
            let rest = match current_env("XDG_DATA_DIRS").filter(|d| !d.is_empty()) {
                Some(dirs) => dirs
                    .split(':')
                    .filter(|d| Path::new(d) != Path::new(&ours))
                    .collect::<Vec<_>>()
                    .join(":"),
                None => {
                    vars.push(var("TERNARYCODE_FISH_XDG_UNSET", "1".into()));
                    XDG_DATA_DIRS_DEFAULT.into()
                }
            };
            let dirs = if rest.is_empty() {
                ours.clone()
            } else {
                format!("{ours}:{rest}")
            };
            vars.push(var("XDG_DATA_DIRS", dirs));
            vars.push(var("TERNARYCODE_FISH_XDG_DIR", ours));
            vars
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_of(shell: &str, current: &[(&str, &str)]) -> HashMap<String, String> {
        let current: HashMap<String, String> = current
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        env(Path::new("/data/shell"), shell, |k| current.get(k).cloned())
            .into_iter()
            .collect()
    }

    #[test]
    fn shell_names() {
        assert_eq!(kind("/bin/zsh"), Some(Kind::Zsh));
        assert_eq!(kind("-zsh"), Some(Kind::Zsh));
        assert_eq!(kind("bash"), Some(Kind::Bash));
        assert_eq!(kind("/opt/homebrew/bin/fish"), Some(Kind::Fish));
        assert_eq!(kind("/bin/sh"), None);
        assert_eq!(kind("/usr/bin/nu"), None);
        assert!(env_of("/bin/dash", &[]).is_empty());
        assert_eq!(kind(r"C:\Program Files\PowerShell\7\pwsh.exe"), Some(Kind::PowerShell));
        assert_eq!(kind(r"C:\Windows\System32\WindowsPowerShell\v1.0\PowerShell.EXE"), Some(Kind::PowerShell));
        assert_eq!(kind(r"C:\Windows\system32\cmd.exe"), None);
        assert_eq!(env_of("pwsh", &[])[ENV_PWSH], "1");
    }

    #[test]
    fn powershell_gets_the_script_as_an_encoded_command() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert!(windows_shell_args("cmd.exe", true).is_empty());
        assert_eq!(windows_shell_args("pwsh.exe", false), ["-NoLogo"]);
        let args = windows_shell_args(r"C:\Program Files\PowerShell\7\pwsh.exe", true);
        assert_eq!(args[..3], ["-NoLogo", "-NoExit", "-EncodedCommand"]);
        // Decodes back to the script.
        let decode = |c: u8| -> u32 {
            (b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/")
                .iter()
                .position(|x| *x == c)
                .unwrap() as u32
        };
        let mut bytes = Vec::new();
        for quad in args[3].as_bytes().chunks(4) {
            let pad = quad.iter().filter(|c| **c == b'=').count();
            let n = quad
                .iter()
                .fold(0u32, |n, c| (n << 6) | if *c == b'=' { 0 } else { decode(*c) });
            bytes.extend_from_slice(&n.to_be_bytes()[1..4 - pad]);
        }
        let utf16: Vec<u16> = bytes.chunks(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect();
        assert_eq!(String::from_utf16(&utf16).unwrap(), POWERSHELL);
        // Well within the 32767 characters of a Windows command line.
        assert!(args[3].len() < 12_000, "{}", args[3].len());
        assert!(POWERSHELL.contains("]133;C") && POWERSHELL.contains("]7;"));
    }

    // POSIX shells: their paths are joined with `/`.
    #[cfg(unix)]
    #[test]
    fn zsh_keeps_the_users_zdotdir() {
        let e = env_of("/bin/zsh", &[]);
        assert_eq!(e["ZDOTDIR"], "/data/shell/zsh");
        assert!(!e.contains_key("TERNARYCODE_ZSH_ZDOTDIR"));
        let e = env_of("zsh", &[("ZDOTDIR", "/home/me/.config/zsh")]);
        assert_eq!(e["ZDOTDIR"], "/data/shell/zsh");
        assert_eq!(e["TERNARYCODE_ZSH_ZDOTDIR"], "/home/me/.config/zsh");
        // Our own directory is not the user's.
        let e = env_of("zsh", &[("ZDOTDIR", "/data/shell/zsh")]);
        assert!(!e.contains_key("TERNARYCODE_ZSH_ZDOTDIR"));
    }

    // POSIX shells: their paths are joined with `/`.
    #[cfg(unix)]
    #[test]
    fn bash_goes_first_in_prompt_command() {
        let boot = r#". '/data/shell/bash/ternarycode.bash';builtin eval "${__tc_late-}""#;
        assert_eq!(env_of("bash", &[])["PROMPT_COMMAND"], boot);
        assert_eq!(
            env_of("bash", &[("PROMPT_COMMAND", "history -a")])["PROMPT_COMMAND"],
            format!("{boot}\nhistory -a")
        );
        let already = format!("{boot}\nhistory -a");
        assert_eq!(
            env_of("bash", &[("PROMPT_COMMAND", &already)])["PROMPT_COMMAND"],
            already
        );
        assert!(env(Path::new("/it's/here"), "bash", |_| None).is_empty());
    }

    #[test]
    fn fish_goes_first_in_xdg_data_dirs() {
        let e = env_of("fish", &[]);
        assert_eq!(
            e["XDG_DATA_DIRS"],
            "/data/shell:/usr/local/share:/usr/share"
        );
        assert_eq!(e["TERNARYCODE_FISH_XDG_DIR"], "/data/shell");
        assert_eq!(e["TERNARYCODE_FISH_XDG_UNSET"], "1");
        let e = env_of(
            "fish",
            &[("XDG_DATA_DIRS", "/nix/share:/data/shell:/usr/share")],
        );
        assert_eq!(e["XDG_DATA_DIRS"], "/data/shell:/nix/share:/usr/share");
        assert!(!e.contains_key("TERNARYCODE_FISH_XDG_UNSET"));
        assert!(env(Path::new("/a:b"), "fish", |_| None).is_empty());
    }

    #[test]
    fn install_writes_and_updates_the_scripts() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("shell");
        install(&dir).unwrap();
        for (rel, content) in FILES {
            assert_eq!(fs::read_to_string(dir.join(rel)).unwrap(), *content);
        }
        let zshenv = dir.join("zsh/.zshenv");
        fs::write(&zshenv, "stale").unwrap();
        install(&dir).unwrap();
        assert_eq!(fs::read_to_string(&zshenv).unwrap(), FILES[0].1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&zshenv).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o644);
        }
        // No temporary files left behind.
        let leftovers: Vec<_> = fs::read_dir(dir.join("zsh"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }
}
