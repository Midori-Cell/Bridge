//! Small helpers: timestamps, private file writes, file-name sanitising.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ts() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// Open for writing, readable by the owner only on Unix. On Windows the per-user
/// profile folder already restricts access to the signed-in user.
pub fn open_private(path: &Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

/// Write via a temporary file and rename, so a crash never leaves a half-written file.
pub fn write_private_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = open_private(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

pub fn random_hex(bytes: usize) -> String {
    use rand::RngCore;
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];
const MAX_NAME_BYTES: usize = 200;

/// Turn a name supplied by the other device into a safe single file name.
///
/// Drops any directory part, replaces characters Windows forbids, removes control
/// characters, avoids Windows device names, and limits the length. Never returns
/// an empty string, "." or "..".
pub fn sanitize_name(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let mut name: String = last
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') { '_' } else { c })
        .collect();
    name = name.trim().trim_end_matches(['.', ' ']).to_string();
    if name.is_empty() || name == "." || name == ".." {
        return "file".to_string();
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        name.insert(0, '_');
    }
    if name.len() > MAX_NAME_BYTES {
        let (stem, ext) = match name.rfind('.') {
            Some(i) if name.len() - i <= 20 => (&name[..i], &name[i..]),
            _ => (name.as_str(), ""),
        };
        let mut cut = MAX_NAME_BYTES - ext.len();
        while !stem.is_char_boundary(cut) {
            cut -= 1;
        }
        name = format!("{}{}", &stem[..cut], ext);
    }
    name
}

/// Reserve a file name inside `dir` that does not exist yet ("a.txt", "a (1).txt", ...).
/// The empty file is created so two transfers cannot pick the same name.
pub fn reserve_unique(dir: &Path, name: &str) -> std::io::Result<PathBuf> {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 0..10_000u32 {
        let candidate = if n == 0 { name.to_string() } else { format!("{stem} ({n}){ext}") };
        let path = dir.join(&candidate);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "too many files with this name"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_directories_and_traversal() {
        assert_eq!(sanitize_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_name("..\\..\\Windows\\system32\\evil.dll"), "evil.dll");
        assert_eq!(sanitize_name("/abs/path/file.txt"), "file.txt");
        assert_eq!(sanitize_name("C:\\Users\\x\\a.txt"), "a.txt");
    }

    #[test]
    fn never_empty_or_dots() {
        for bad in ["", ".", "..", "...", "   ", "/", "a/..", "\0\0"] {
            let out = sanitize_name(bad);
            assert!(!out.is_empty() && out != "." && out != "..", "{bad:?} -> {out:?}");
        }
    }

    #[test]
    fn windows_rules() {
        assert_eq!(sanitize_name("a<b>c:d\"e|f?g*h.txt"), "a_b_c_d_e_f_g_h.txt");
        assert_eq!(sanitize_name("CON"), "_CON");
        assert_eq!(sanitize_name("nul.txt"), "_nul.txt");
        assert_eq!(sanitize_name("com1.tar.gz"), "_com1.tar.gz");
        assert_eq!(sanitize_name("report. "), "report");
        assert_eq!(sanitize_name("notcon.txt"), "notcon.txt");
    }

    #[test]
    fn control_chars_and_unicode() {
        assert_eq!(sanitize_name("a\nb\tc\u{7f}.txt"), "abc.txt");
        assert_eq!(sanitize_name("Übung ß \u{1F600}.pdf"), "Übung ß \u{1F600}.pdf");
    }

    #[test]
    fn long_names_keep_extension_and_char_boundary() {
        let long = format!("{}.jpeg", "é".repeat(300));
        let out = sanitize_name(&long);
        assert!(out.len() <= 200 && out.ends_with(".jpeg"));
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn unique_names_do_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let a = reserve_unique(dir.path(), "a.txt").unwrap();
        let b = reserve_unique(dir.path(), "a.txt").unwrap();
        let c = reserve_unique(dir.path(), "noext").unwrap();
        let d = reserve_unique(dir.path(), "noext").unwrap();
        assert_eq!(a.file_name().unwrap(), "a.txt");
        assert_eq!(b.file_name().unwrap(), "a (1).txt");
        assert_eq!(c.file_name().unwrap(), "noext");
        assert_eq!(d.file_name().unwrap(), "noext (1)");
    }
}
