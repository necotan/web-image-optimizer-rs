use std::fs;
use std::path::{Path, PathBuf};

use crate::error::ScanError;

pub fn scan_dir(dir: &Path) -> Result<Vec<PathBuf>, ScanError> {
    if !dir.is_dir() {
        return Err(ScanError::NotFound(dir.to_path_buf()));
    }
    let read_error = |source| ScanError::Read {
        path: dir.to_path_buf(),
        source,
    };

    let mut files = Vec::new();
    for entry in fs::read_dir(dir).map_err(read_error)? {
        let entry = entry.map_err(read_error)?;
        let is_file = entry.file_type().map_err(read_error)?.is_file();
        if is_file && is_jpeg(&entry.path()) {
            files.push(entry.path());
        }
    }
    files.sort();
    Ok(files)
}

fn is_jpeg(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wio-scan-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lists_only_direct_jpegs_sorted_by_name() {
        let dir = temp_dir("list");
        for name in ["b.JPG", "a.jpeg", "c.jpg", "note.txt", "d.png"] {
            fs::write(dir.join(name), b"x").unwrap();
        }
        fs::create_dir_all(dir.join("export")).unwrap();
        fs::write(dir.join("export").join("old.jpg"), b"x").unwrap();
        fs::create_dir_all(dir.join("folder.jpg")).unwrap();

        let names: Vec<String> = scan_dir(&dir)
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.jpeg", "b.JPG", "c.jpg"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_folder_is_an_error() {
        let dir = temp_dir("missing").join("nothing");
        assert!(matches!(scan_dir(&dir), Err(ScanError::NotFound(_))));
    }
}
