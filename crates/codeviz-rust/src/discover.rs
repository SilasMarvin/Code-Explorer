use std::path::{Path, PathBuf};

use serde::Deserialize;
use walkdir::{DirEntry, WalkDir};

use crate::Error;

/// A Cargo package with a `src/` directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// The package name as a Rust identifier (`-` replaced by `_`).
    pub crate_name: String,
    pub src_dir: PathBuf,
}

#[derive(Debug)]
pub(crate) struct SourceFile {
    pub(crate) path: PathBuf,
    /// Module path including the crate name.
    pub(crate) module: Vec<String>,
}

#[derive(Deserialize)]
struct Manifest {
    package: Option<ManifestPackage>,
}

#[derive(Deserialize)]
struct ManifestPackage {
    name: String,
}

pub(crate) fn packages(root: &Path) -> Result<Vec<Package>, Error> {
    let mut packages = Vec::new();
    for entry in walk(root) {
        let entry = entry.map_err(|source| Error::Walk {
            root: root.to_path_buf(),
            source,
        })?;
        if entry.file_name() != "Cargo.toml" {
            continue;
        }
        let manifest_path = entry.path();
        let Some(dir) = manifest_path.parent() else {
            continue;
        };
        let src_dir = dir.join("src");
        if !src_dir.is_dir() {
            continue;
        }
        let text = std::fs::read_to_string(manifest_path).map_err(|source| Error::Io {
            path: manifest_path.to_path_buf(),
            source,
        })?;
        let manifest: Manifest = toml::from_str(&text).map_err(|source| Error::Manifest {
            path: manifest_path.to_path_buf(),
            source,
        })?;
        if let Some(package) = manifest.package {
            packages.push(Package {
                crate_name: package.name.replace('-', "_"),
                src_dir,
            });
        }
    }
    packages.sort_by(|a, b| a.crate_name.cmp(&b.crate_name));
    Ok(packages)
}

/// Every `.rs` file under the package's `src/`. Directories named `tests` hold
/// `#[cfg(test)]` modules by convention and are skipped unless `include_tests`.
pub(crate) fn source_files(
    package: &Package,
    include_tests: bool,
) -> Result<Vec<SourceFile>, Error> {
    let mut files = Vec::new();
    for entry in walk(&package.src_dir) {
        let entry = entry.map_err(|source| Error::Walk {
            root: package.src_dir.clone(),
            source,
        })?;
        let path = entry.path();
        if !entry.file_type().is_file() || path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        let Ok(relative) = path.strip_prefix(&package.src_dir) else {
            continue;
        };
        if !include_tests
            && relative
                .parent()
                .is_some_and(|dir| dir.iter().any(|s| s == "tests"))
        {
            continue;
        }
        files.push(SourceFile {
            module: module_path(&package.crate_name, relative),
            path: path.to_path_buf(),
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// `src/a/b.rs` and `src/a/b/mod.rs` are `crate::a::b`; `lib.rs` and `main.rs` are the root.
fn module_path(crate_name: &str, relative: &Path) -> Vec<String> {
    let mut module = vec![crate_name.to_owned()];
    let segments: Vec<String> = relative
        .with_extension("")
        .iter()
        .map(|segment| segment.to_string_lossy().into_owned())
        .collect();
    let is_root = matches!(segments.as_slice(), [only] if only == "lib" || only == "main");
    if !is_root {
        module.extend(segments);
        if module.last().is_some_and(|last| last == "mod") {
            module.pop();
        }
    }
    module
}

fn walk(root: &Path) -> impl Iterator<Item = walkdir::Result<DirEntry>> {
    WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            entry.depth() == 0 || !(name.starts_with('.') || name == "target")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_paths() {
        let cases = [
            ("lib.rs", vec!["app"]),
            ("main.rs", vec!["app"]),
            ("store.rs", vec!["app", "store"]),
            ("services/mod.rs", vec!["app", "services"]),
            ("services/users.rs", vec!["app", "services", "users"]),
        ];
        for (file, expected) in cases {
            assert_eq!(module_path("app", Path::new(file)), expected, "{file}");
        }
    }
}
