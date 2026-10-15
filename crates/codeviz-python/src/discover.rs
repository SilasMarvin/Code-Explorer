//! Finds Python source files and names the module each one defines.

use std::path::{Path, PathBuf};

use walkdir::{DirEntry, WalkDir};

use crate::Error;

/// Directories that hold tooling, environments or build output, never analyzed source.
const SKIPPED_DIRS: &[&str] = &[
    "__pycache__",
    "node_modules",
    "site-packages",
    "venv",
    "env",
    "build",
    "dist",
    "target",
];

#[derive(Debug, Clone)]
pub(crate) struct SourceFile {
    pub(crate) path: PathBuf,
    /// POSIX path relative to the analyzed root.
    pub(crate) uri: String,
    /// Dotted module name, e.g. `shop.routes.orders`.
    pub(crate) module: String,
    /// An `__init__.py`, i.e. the module is a package.
    pub(crate) is_package: bool,
}

pub(crate) fn source_files(root: &Path) -> Result<Vec<SourceFile>, Error> {
    let mut files = Vec::new();
    let walker = WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(is_source_dir);
    for entry in walker {
        let entry = entry.map_err(|source| Error::Walk {
            root: root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if !entry.file_type().is_file() || path.extension().is_none_or(|ext| ext != "py") {
            continue;
        }
        let Some(module) = module_name(path) else {
            continue;
        };
        let relative = path.strip_prefix(root).unwrap_or(path);
        files.push(SourceFile {
            uri: posix(relative),
            is_package: path.file_name().is_some_and(|name| name == "__init__.py"),
            module,
            path: path.to_path_buf(),
        });
    }
    Ok(files)
}

fn is_source_dir(entry: &DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    entry.depth() == 0
        || !(name.starts_with('.')
            || (entry.file_type().is_dir() && SKIPPED_DIRS.contains(&&*name)))
}

/// Names a module the way `import` would: the file's stem, prefixed by every enclosing
/// directory that is a package (has an `__init__.py`). The walk may leave the analyzed
/// root so that analyzing a subpackage still yields fully qualified, stable names.
fn module_name(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let mut segments = Vec::new();
    if stem != "__init__" {
        segments.push(stem.to_owned());
    }
    let mut dir = path.parent();
    while let Some(package) = dir.filter(|dir| dir.join("__init__.py").is_file()) {
        segments.push(package.file_name()?.to_str()?.to_owned());
        dir = package.parent();
    }
    segments.reverse();
    (!segments.is_empty()).then(|| segments.join("."))
}

fn posix(path: &Path) -> String {
    path.iter()
        .map(|segment| segment.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_modules_by_enclosing_packages() -> anyhow::Result<()> {
        let dir =
            std::env::temp_dir().join(format!("codeviz-python-discover-{}", std::process::id()));
        let package = dir.join("app/sub");
        std::fs::create_dir_all(&package)?;
        for file in [
            "app/__init__.py",
            "app/sub/__init__.py",
            "app/sub/mod.py",
            "script.py",
        ] {
            std::fs::write(dir.join(file), "")?;
        }

        let files = source_files(&dir)?;
        let names: Vec<_> = files
            .iter()
            .map(|f| (f.uri.as_str(), f.module.as_str()))
            .collect();
        std::fs::remove_dir_all(&dir)?;

        assert_eq!(
            names,
            [
                ("app/__init__.py", "app"),
                ("app/sub/__init__.py", "app.sub"),
                ("app/sub/mod.py", "app.sub.mod"),
                ("script.py", "script"),
            ]
        );
        Ok(())
    }
}
