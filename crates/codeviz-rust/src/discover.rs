//! Finds the crates under a directory, the workspaces rust-analyzer loads them from,
//! and the source files of each crate.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use walkdir::{DirEntry, WalkDir};

use crate::Error;

/// A crate: a package's library or one of its binaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crate {
    /// The crate name as a Rust identifier (`-` replaced by `_`). A binary named like
    /// its package's library shares the library's name, and so its modules.
    pub name: String,
    /// Its root file: `src/lib.rs`, `src/main.rs`, `src/bin/*.rs`, or what the
    /// manifest names.
    pub root: PathBuf,
    /// Whether it is its package's library.
    pub library: bool,
    /// Its package's name, as written.
    pub package_name: String,
    /// Where its package's `Cargo.toml` is.
    pub package: PathBuf,
    /// Where the workspace rust-analyzer loads it from is: the package's own directory
    /// when it is in no workspace.
    pub workspace: PathBuf,
}

#[derive(Debug)]
pub(crate) struct SourceFile {
    pub(crate) path: PathBuf,
    /// Module path including the crate name.
    pub(crate) module: Vec<String>,
}

/// What was found under the analyzed directory.
#[derive(Debug, Default)]
pub(crate) struct Discovery {
    pub(crate) crates: Vec<Crate>,
    /// What was skipped, for the person who ran the analysis.
    pub(crate) notes: Vec<String>,
}

#[derive(Deserialize)]
struct Manifest {
    package: Option<ManifestPackage>,
    workspace: Option<ManifestWorkspace>,
    lib: Option<Target>,
    #[serde(default)]
    bin: Vec<Target>,
    #[serde(default)]
    dependencies: BTreeMap<String, toml::Value>,
    #[serde(default, rename = "dev-dependencies")]
    dev_dependencies: BTreeMap<String, toml::Value>,
    #[serde(default, rename = "build-dependencies")]
    build_dependencies: BTreeMap<String, toml::Value>,
}

impl Manifest {
    /// The directories of the packages it depends on by path.
    fn path_dependencies<'m>(&'m self, dir: &'m Path) -> impl Iterator<Item = PathBuf> + 'm {
        let shared = self.workspace.as_ref().map(|w| &w.dependencies);
        [
            &self.dependencies,
            &self.dev_dependencies,
            &self.build_dependencies,
        ]
        .into_iter()
        .chain(shared)
        .flat_map(BTreeMap::values)
        .filter_map(|dependency| dependency.get("path")?.as_str())
        .map(move |path| normalize(&dir.join(path)))
    }
}

/// `a/b/../c` → `a/c`, without touching the file system.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

#[derive(Deserialize)]
struct ManifestWorkspace {
    #[serde(default)]
    members: Vec<String>,
    #[serde(default)]
    dependencies: BTreeMap<String, toml::Value>,
    #[serde(default)]
    exclude: Vec<String>,
}

#[derive(Deserialize)]
struct ManifestPackage {
    name: String,
    autobins: Option<bool>,
    metadata: Option<toml::Value>,
}

#[derive(Deserialize)]
struct Target {
    name: Option<String>,
    path: Option<String>,
}

/// Every crate of every package under `root`, except packages a workspace under
/// `root` does not count among its members, and those under a `tests` directory
/// unless `include_tests`.
pub(crate) fn crates(root: &Path, include_tests: bool) -> Result<Discovery, Error> {
    let mut discovery = Discovery::default();
    let mut manifests = BTreeMap::new();
    for entry in walk(root) {
        let entry = entry.map_err(|source| Error::Walk {
            root: root.to_path_buf(),
            source,
        })?;
        if entry.file_name() != "Cargo.toml" || !entry.file_type().is_file() {
            continue;
        }
        let Some(dir) = entry.path().parent() else {
            continue;
        };
        match read_manifest(entry.path()) {
            Ok(manifest) => {
                manifests.insert(dir.to_path_buf(), manifest);
            }
            // One broken manifest, often a test fixture, is no reason to analyze nothing.
            Err(error) => discovery.notes.push(format!("skipped {error}")),
        }
    }
    // Cargo counts a workspace's path dependencies among its members.
    let depended: Vec<PathBuf> = manifests
        .iter()
        .flat_map(|(dir, manifest)| manifest.path_dependencies(dir))
        .collect();
    let mut skipped = Vec::new();
    let mut fuzz = Vec::new();
    for (dir, manifest) in &manifests {
        let Some(package) = &manifest.package else {
            continue;
        };
        let workspace = workspace_of(dir, &manifests);
        let outside = matches!(&workspace, Some((workspace, false)) if workspace.starts_with(root))
            && !depended.contains(dir);
        // A package under `tests/` is a fixture, such as this repository's own.
        let fixture = !include_tests
            && dir
                .strip_prefix(root)
                .is_ok_and(|inside| inside.iter().any(|s| s == "tests"));
        if outside || fixture {
            skipped.push(dir.strip_prefix(root).unwrap_or(dir).display().to_string());
            continue;
        }
        if dir != root && is_fuzz(dir, package, manifest) {
            fuzz.push(dir.strip_prefix(root).unwrap_or(dir).display().to_string());
            continue;
        }
        let workspace = workspace.map_or_else(|| dir.clone(), |(dir, _)| dir);
        discovery
            .crates
            .extend(
                targets(dir, package, manifest)
                    .into_iter()
                    .map(|(name, root, library)| Crate {
                        name,
                        root,
                        library,
                        package_name: package.name.clone(),
                        package: dir.clone(),
                        workspace: workspace.clone(),
                    }),
            );
    }
    if !fuzz.is_empty() {
        discovery.notes.push(format!(
            "skipped cargo-fuzz targets (point codeviz at one to analyze it): {}",
            fuzz.join(", ")
        ));
    }
    if !skipped.is_empty() {
        let more = skipped.len().saturating_sub(3);
        skipped.truncate(3);
        let mut line = format!(
            "skipped test fixtures and packages their workspace leaves out: {}",
            skipped.join(", ")
        );
        if more > 0 {
            let _ = write!(line, " and {more} more");
        }
        discovery.notes.push(line);
    }
    discovery
        .crates
        .sort_by(|a, b| (&a.name, &a.root).cmp(&(&b.name, &b.root)));
    Ok(discovery)
}

/// The package `dir` is inside of, when it holds no manifest itself: the nearest
/// directory above it with a `[package]` manifest.
pub(crate) fn enclosing_package(dir: &Path) -> Option<&Path> {
    dir.ancestors().skip(1).find(|ancestor| {
        read_manifest(&ancestor.join("Cargo.toml")).is_ok_and(|m| m.package.is_some())
    })
}

/// A cargo-fuzz package: marked so in its metadata, or a `fuzz` directory that
/// depends on `libfuzzer-sys`.
fn is_fuzz(dir: &Path, package: &ManifestPackage, manifest: &Manifest) -> bool {
    let marked = package
        .metadata
        .as_ref()
        .and_then(|m| m.get("cargo-fuzz"))
        .and_then(toml::Value::as_bool)
        == Some(true);
    marked
        || (dir.file_name().is_some_and(|n| n == "fuzz")
            && manifest.dependencies.contains_key("libfuzzer-sys"))
}

fn read_manifest(path: &Path) -> Result<Manifest, Error> {
    let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&text).map_err(|source| Error::Manifest {
        path: path.to_path_buf(),
        source,
    })
}

/// The workspace the package in `dir` belongs to, and whether it is a member, from
/// the manifests found or those above them: the nearest `[workspace]` at or above it.
fn workspace_of(dir: &Path, found: &BTreeMap<PathBuf, Manifest>) -> Option<(PathBuf, bool)> {
    for ancestor in dir.ancestors() {
        let read = (!found.contains_key(ancestor))
            .then(|| read_manifest(&ancestor.join("Cargo.toml")).ok())
            .flatten();
        let Some(manifest) = found.get(ancestor).or(read.as_ref()) else {
            continue;
        };
        let Some(workspace) = &manifest.workspace else {
            continue;
        };
        let Ok(relative) = dir.strip_prefix(ancestor) else {
            continue;
        };
        let matches = |patterns: &[String]| patterns.iter().any(|p| glob(p, relative));
        let member = relative.as_os_str().is_empty()
            || (matches(&workspace.members) && !excluded(&workspace.exclude, relative));
        return Some((ancestor.to_path_buf(), member));
    }
    None
}

/// Whether `relative` is under one of the excluded paths.
fn excluded(exclude: &[String], relative: &Path) -> bool {
    exclude
        .iter()
        .any(|path| relative.starts_with(path.trim_end_matches('/')))
}

/// A workspace member pattern against a path, component by component; `*` and `?`
/// match within a component.
fn glob(pattern: &str, path: &Path) -> bool {
    let pattern: Vec<&str> = pattern
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    let path: Vec<String> = path
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    pattern.len() == path.len()
        && pattern
            .iter()
            .zip(&path)
            .all(|(p, s)| wildcard(p.as_bytes(), s.as_bytes()))
}

fn wildcard(pattern: &[u8], text: &[u8]) -> bool {
    match (pattern.split_first(), text.split_first()) {
        (None, None) => true,
        (Some((b'*', rest)), _) => {
            wildcard(rest, text)
                || text
                    .split_first()
                    .is_some_and(|(_, t)| wildcard(pattern, t))
        }
        (Some((b'?', rest)), Some((_, t))) => wildcard(rest, t),
        (Some((p, rest)), Some((c, t))) => p == c && wildcard(rest, t),
        _ => false,
    }
}

/// A package's library and binaries, as Cargo finds them: `[lib]` and `[[bin]]`
/// paths, `src/lib.rs`, `src/main.rs`, `src/bin/*.rs` and `src/bin/*/main.rs`.
fn targets(
    dir: &Path,
    package: &ManifestPackage,
    manifest: &Manifest,
) -> Vec<(String, PathBuf, bool)> {
    let ident = |name: &str| name.replace('-', "_");
    let lib_name = manifest
        .lib
        .as_ref()
        .and_then(|lib| lib.name.as_deref())
        .map_or_else(|| ident(&package.name), ident);
    let mut found: Vec<(String, PathBuf, bool)> = Vec::new();
    let lib = manifest
        .lib
        .as_ref()
        .and_then(|lib| lib.path.as_deref())
        .map_or_else(|| dir.join("src/lib.rs"), |path| dir.join(path));
    if lib.is_file() {
        found.push((lib_name, lib, true));
    }
    for bin in &manifest.bin {
        let path = match (&bin.path, &bin.name) {
            (Some(path), _) => dir.join(path),
            (None, Some(name)) if *name == package.name => dir.join("src/main.rs"),
            (None, Some(name)) => {
                let single = dir.join("src/bin").join(format!("{name}.rs"));
                if single.is_file() {
                    single
                } else {
                    dir.join("src/bin").join(name).join("main.rs")
                }
            }
            (None, None) => continue,
        };
        let name = bin.name.as_deref().unwrap_or(&package.name);
        found.push((ident(name), path, false));
    }
    if package.autobins != Some(false) {
        found.push((ident(&package.name), dir.join("src/main.rs"), false));
        let bins = std::fs::read_dir(dir.join("src/bin")).into_iter().flatten();
        for entry in bins.flatten() {
            let path = entry.path();
            let name = path.file_stem().map(|s| s.to_string_lossy().into_owned());
            let root = if path.is_dir() {
                path.join("main.rs")
            } else if path.extension().is_some_and(|e| e == "rs") {
                path.clone()
            } else {
                continue;
            };
            if let Some(name) = name {
                found.push((ident(&name), root, false));
            }
        }
    }
    // A target named twice, as explicit and as found, is the explicit one.
    let mut seen = Vec::new();
    found.retain(|(_, root, _)| {
        let new = root.is_file() && !seen.contains(root);
        seen.push(root.clone());
        new
    });
    found
}

/// Every `.rs` file of every crate, under the module path it has there. A crate holds
/// its root and the files below its root's directory, except where another crate's
/// root directory, another package, or a `tests` directory (unless `include_tests`)
/// begins. A file under several crates' directories goes to the one deepest down,
/// a library before a binary.
pub(crate) fn source_files(
    crates: &[Crate],
    include_tests: bool,
) -> Result<Vec<SourceFile>, Error> {
    let roots: BTreeMap<&Path, &Crate> = crates.iter().map(|c| (c.root.as_path(), c)).collect();
    let mut files: BTreeMap<PathBuf, (usize, bool, Vec<String>)> = BTreeMap::new();
    for krate in crates {
        let is_lib = krate.library;
        files.insert(
            krate.root.clone(),
            (usize::MAX, true, vec![krate.name.clone()]),
        );
        let Some(dir) = krate.root.parent() else {
            continue;
        };
        let depth = dir.components().count();
        let entries = WalkDir::new(dir)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|entry| entry.depth() == 0 || keep(entry, &krate.package, include_tests));
        for entry in entries {
            let entry = entry.map_err(|source| Error::Walk {
                root: dir.to_path_buf(),
                source,
            })?;
            let path = entry.path();
            if !entry.file_type().is_file()
                || path.extension().is_none_or(|ext| ext != "rs")
                || roots.contains_key(path)
            {
                continue;
            }
            let Ok(relative) = path.strip_prefix(dir) else {
                continue;
            };
            let claim = (depth, is_lib);
            if files
                .get(path)
                .is_some_and(|(d, lib, _)| (*d, *lib) >= claim)
            {
                continue;
            }
            files.insert(
                path.to_path_buf(),
                (depth, is_lib, module_path(&krate.name, relative)),
            );
        }
    }
    Ok(files
        .into_iter()
        .map(|(path, (_, _, module))| SourceFile { path, module })
        .collect())
}

/// Whether a crate's walk enters `entry`: not another package, nor what Cargo builds
/// apart (build scripts, examples, benches, integration tests), nor `tests` modules.
fn keep(entry: &DirEntry, package: &Path, include_tests: bool) -> bool {
    let name = entry.file_name().to_string_lossy();
    let path = entry.path();
    if entry.file_type().is_dir() {
        let at_package = path.parent() == Some(package);
        !(name.starts_with('.')
            || name == "target"
            || (name == "tests" && !include_tests)
            || (at_package && matches!(name.as_ref(), "examples" | "benches" | "tests"))
            || path.join("Cargo.toml").is_file())
    } else {
        !(name == "build.rs" && path.parent() == Some(package))
    }
}

/// `a/b.rs` and `a/b/mod.rs` under a crate root's directory are `crate::a::b`.
fn module_path(crate_name: &str, relative: &Path) -> Vec<String> {
    let mut module = vec![crate_name.to_owned()];
    module.extend(
        relative
            .with_extension("")
            .iter()
            .map(|segment| segment.to_string_lossy().into_owned()),
    );
    if module.len() > 1 && module.last().is_some_and(|last| last == "mod") {
        module.pop();
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
            ("store.rs", vec!["app", "store"]),
            ("services/mod.rs", vec!["app", "services"]),
            ("services/users.rs", vec!["app", "services", "users"]),
            ("services/lib.rs", vec!["app", "services", "lib"]),
        ];
        for (file, expected) in cases {
            assert_eq!(module_path("app", Path::new(file)), expected, "{file}");
        }
    }

    #[test]
    fn member_globs() {
        assert!(glob("crates/*", Path::new("crates/core")));
        assert!(glob("helix-*", Path::new("helix-term")));
        assert!(glob("./app", Path::new("app")));
        assert!(!glob("crates/*", Path::new("crates/core/inner")));
        assert!(!glob("helix-*", Path::new("xtask")));
    }
}
