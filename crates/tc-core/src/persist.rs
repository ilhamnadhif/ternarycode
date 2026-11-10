//! Private, atomic file writes (FR-L5): data goes to a temporary file in the same folder, is
//! flushed to disk, and replaces the target with a rename, so a crash leaves either the old or
//! the new file, never half of one. Files are readable by the user only.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Creates `dir` (and parents) readable by the user only.
pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

pub fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    // A symbolic link (a dotfiles repository, say) stays one: the file it points at is written.
    let resolved;
    let path = if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        resolved = fs::canonicalize(path)?;
        resolved.as_path()
    } else {
        path
    };
    // A file that exists keeps its permissions; a new one is readable by the user only.
    #[cfg(unix)]
    let keep_mode = fs::metadata(path).ok().map(|m| m.permissions().mode() & 0o7777);
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        opts.mode(0o600);
        let mut f = opts.open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        #[cfg(unix)]
        if let Some(mode) = keep_mode {
            fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
        }
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

pub fn save_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let data = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    write_private(path, &data)
}

/// Loads a JSON file. A missing file gives the default. A file that does not fully read (a
/// field from a newer version, a hand edit gone wrong) keeps what it can: each field and list
/// item that reads is kept, the rest takes its default; the file as it was is kept aside as
/// `<name>.broken[-N]` so nothing is lost.
pub fn load_json<T: DeserializeOwned + Serialize + Default>(path: &Path) -> T {
    let Ok(data) = fs::read(path) else {
        return T::default();
    };
    match serde_json::from_slice(&data) {
        Ok(v) => v,
        Err(_) => {
            keep_aside(path);
            let kept: T = serde_json::from_slice::<serde_json::Value>(&data)
                .ok()
                .and_then(|v| salvage::<T>(v))
                .unwrap_or_default();
            // What was kept is the file from now on (whoever reads it next gets the same).
            let _ = save_json(path, &kept);
            kept
        }
    }
}

/// Renames a damaged file to a name not used yet.
fn keep_aside(path: &Path) {
    let mut n = 0;
    loop {
        let mut name = path.as_os_str().to_owned();
        name.push(if n == 0 { ".broken".to_string() } else { format!(".broken-{n}") });
        let target = std::path::PathBuf::from(name);
        if !target.exists() {
            let _ = fs::rename(path, target);
            return;
        }
        n += 1;
        if n > 100 {
            return;
        }
    }
}

/// The default value with each top-level field of `found` put in when it reads, and of a list,
/// each item that reads.
fn salvage<T: DeserializeOwned + Serialize + Default>(found: serde_json::Value) -> Option<T> {
    use serde_json::Value;
    let Value::Object(fields) = found else { return None };
    let mut base = serde_json::to_value(T::default()).ok()?;
    for (key, value) in fields {
        let fits = |base: &Value, v: Value| {
            let mut candidate = base.clone();
            candidate.as_object_mut()?.insert(key.clone(), v);
            serde_json::from_value::<T>(candidate.clone()).ok().map(|_| candidate)
        };
        if let Some(next) = fits(&base, value.clone()) {
            base = next;
        } else if let Value::Array(items) = value {
            // One bad item must not cost the others (a project, a preset).
            let mut kept = Vec::new();
            for item in items {
                kept.push(item);
                if fits(&base, Value::Array(kept.clone())).is_none() {
                    kept.pop();
                }
            }
            if let Some(next) = fits(&base, Value::Array(kept)) {
                base = next;
            }
        }
    }
    serde_json::from_value(base).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Settings;

    #[test]
    fn writes_privately_and_atomically_and_survives_damage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/settings.json");
        let s = Settings {
            font_size: 17.0,
            ..Settings::default()
        };
        save_json(&path, &s).unwrap();
        #[cfg(unix)]
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let back: Settings = load_json(&path);
        assert_eq!(back.font_size, 17.0);
        // No temporary files left behind.
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
        // A damaged file: defaults, and the bad file kept aside.
        fs::write(&path, b"{ not json").unwrap();
        let fallback: Settings = load_json(&path);
        assert_eq!(fallback.font_size, Settings::default().font_size);
        assert!(path.with_file_name("settings.json.broken").exists());
        // One bad field: the others are kept; the earlier broken file stays.
        fs::write(&path, br#"{"font_size": 15.0, "theme": "plaid", "presets": [{"id": "x"}, {"id": "y", "name": "Y", "command": "y"}]}"#).unwrap();
        let partly: Settings = load_json(&path);
        assert_eq!(partly.font_size, 15.0);
        assert_eq!(partly.theme, Settings::default().theme);
        assert_eq!(partly.presets.len(), 1);
        assert!(path.with_file_name("settings.json.broken-1").exists());
        // Missing file: defaults.
        let missing: Settings = load_json(&dir.path().join("nope.json"));
        assert_eq!(missing, Settings::default());
    }
}
