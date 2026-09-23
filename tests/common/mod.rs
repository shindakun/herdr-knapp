#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const REPO: &str = env!("CARGO_MANIFEST_DIR");

/// A fresh, empty temp directory unique to this test process and `name`.
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("knapp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::canonicalize(dir).expect("canonical temp dir")
}

/// A temp copy of `fixtures/<fixture>`, with every mtime a minute old so the
/// cache's two-second rule does not force reparsing.
pub fn temp_copy(fixture: &str, name: &str) -> PathBuf {
    let dir = temp_dir(name);
    copy_dir(&Path::new(REPO).join("fixtures").join(fixture), &dir);
    age(&dir);
    dir
}

pub fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create dir");
    for entry in std::fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("entry");
        let dest = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).expect("copy");
        }
    }
}

/// Sets every file's mtime under `dir` to one minute ago.
pub fn age(dir: &Path) {
    let then = SystemTime::now() - Duration::from_secs(60);
    for entry in std::fs::read_dir(dir).expect("read dir") {
        let entry = entry.expect("entry");
        if entry.file_type().expect("type").is_dir() {
            age(&entry.path());
        } else {
            set_mtime(&entry.path(), then);
        }
    }
}

pub fn set_mtime(path: &Path, when: SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .expect("open for mtime")
        .set_modified(when)
        .expect("set mtime");
}
