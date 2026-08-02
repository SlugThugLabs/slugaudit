use ignore::WalkBuilder;
use std::path::{Path, PathBuf};
use thiserror::Error;

const ACTIVATION_RELATIVE_DIR: &str = ".planning/slugaudit";
const EXCLUDED_COMPONENT: &str = ".git";
/// Bytes sampled from the start of a file to decide binary vs. text, same
/// heuristic class as git/ripgrep: a NUL byte in the sample means binary.
const BINARY_SNIFF_BYTES: usize = 8_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Indexed,
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFile {
    pub relative_path: String,
    pub absolute_path: PathBuf,
    pub kind: FileKind,
}

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("failed to walk project files: {0}")]
    Walk(#[from] ignore::Error),
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("discovered path {0} is not relative to the project root")]
    NotRelative(PathBuf),
}

fn is_excluded(relative: &Path) -> bool {
    if relative.starts_with(ACTIVATION_RELATIVE_DIR) {
        return true;
    }
    relative
        .components()
        .any(|component| component.as_os_str() == EXCLUDED_COMPONENT)
}

fn sniff_kind(path: &Path) -> Result<FileKind, DiscoveryError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|source| DiscoveryError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let mut buffer = vec![0_u8; BINARY_SNIFF_BYTES];
    let read = file
        .read(&mut buffer)
        .map_err(|source| DiscoveryError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    if buffer[..read].contains(&0) {
        Ok(FileKind::Binary)
    } else {
        Ok(FileKind::Indexed)
    }
}

/// Walks the project root for non-binary and binary files alike, honoring
/// standard ignore files, never descending into VCS internals or
/// SlugAudit's own activation directory, and never following symlinks that
/// leave the project root. Returns paths in deterministic sorted order.
///
/// # Errors
///
/// Returns an error if the walk itself fails (e.g. an unreadable
/// directory), a discovered path isn't relative to `root`, or a file can't
/// be read to sniff whether it's binary.
pub fn discover(root: &Path) -> Result<Vec<DiscoveredFile>, DiscoveryError> {
    // `standard_filters` sets several flags at once, including `hidden`; it
    // must run before the explicit `hidden(false)` override or it silently
    // wins and dotfiles/dotdirs (e.g. `.github/`) vanish from the walk.
    let walker = WalkBuilder::new(root)
        .standard_filters(true)
        .follow_links(false)
        .hidden(false)
        .build();

    let mut discovered = Vec::new();
    for entry in walker {
        let entry = entry?;
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let absolute_path = entry.path().to_path_buf();
        let relative_path = absolute_path
            .strip_prefix(root)
            .map_err(|_| DiscoveryError::NotRelative(absolute_path.clone()))?;
        if is_excluded(relative_path) {
            continue;
        }
        let kind = sniff_kind(&absolute_path)?;
        discovered.push(DiscoveredFile {
            relative_path: relative_path.to_string_lossy().replace('\\', "/"),
            absolute_path,
            kind,
        });
    }
    discovered.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(discovered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(root: &Path, relative: &str, content: &[u8]) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(path, content).expect("write fixture file");
    }

    #[test]
    fn discovers_source_config_and_doc_files_in_sorted_order() {
        let directory = tempfile::tempdir().expect("temp dir");
        let root = directory.path();
        write(root, "src/main.rs", b"fn main() {}");
        write(root, "README.md", b"# Title");
        write(root, ".github/workflows/ci.yml", b"name: ci");

        let files = discover(root).expect("discover");
        let paths: Vec<&str> = files
            .iter()
            .map(|file| file.relative_path.as_str())
            .collect();
        assert_eq!(
            paths,
            vec![".github/workflows/ci.yml", "README.md", "src/main.rs"]
        );
    }

    #[test]
    fn classifies_binary_files_without_dropping_them() {
        let directory = tempfile::tempdir().expect("temp dir");
        let root = directory.path();
        write(
            root,
            "image.png",
            &[0x89, b'P', b'N', b'G', 0x00, 0x01, 0x02],
        );

        let files = discover(root).expect("discover");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].kind, FileKind::Binary);
    }

    #[test]
    fn excludes_the_activation_directory() {
        let directory = tempfile::tempdir().expect("temp dir");
        let root = directory.path();
        write(root, "src/lib.rs", b"pub fn lib() {}");
        write(root, ".planning/slugaudit/project.db", b"sqlite-bytes");

        let files = discover(root).expect("discover");
        let paths: Vec<&str> = files
            .iter()
            .map(|file| file.relative_path.as_str())
            .collect();
        assert_eq!(paths, vec!["src/lib.rs"]);
    }

    #[test]
    fn excludes_git_internals() {
        let directory = tempfile::tempdir().expect("temp dir");
        let root = directory.path();
        write(root, "src/lib.rs", b"pub fn lib() {}");
        write(root, ".git/HEAD", b"ref: refs/heads/main");

        let files = discover(root).expect("discover");
        let paths: Vec<&str> = files
            .iter()
            .map(|file| file.relative_path.as_str())
            .collect();
        assert_eq!(paths, vec!["src/lib.rs"]);
    }

    #[test]
    fn honors_gitignore_content() {
        let directory = tempfile::tempdir().expect("temp dir");
        let root = directory.path();
        write(root, ".gitignore", b"ignored.txt\n");
        write(root, "ignored.txt", b"should not appear");
        write(root, "kept.txt", b"should appear");

        let files = discover(root).expect("discover");
        let paths: Vec<&str> = files
            .iter()
            .map(|file| file.relative_path.as_str())
            .collect();
        // .gitignore itself is a legitimate config file and stays indexed;
        // only the file it excludes, ignored.txt, is missing.
        assert_eq!(paths, vec![".gitignore", "kept.txt"]);
    }
}
