//! Analyses of directories kept between runs, so `codeviz view DIR` opens at once the
//! second time. An analysis is kept for the commit the directory is at and, when it
//! has changes on top, for exactly those changes; anything else analyzes afresh.

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    process::Command,
};

use codeviz_ir::Document;

use crate::{ir::Source, state::fnv};

/// How many analyses of one directory are kept, newest first.
const KEPT: usize = 4;

/// The analysis of `dir`, from the cache when its source has not changed since, else
/// made now and kept.
pub(crate) fn analyze(dir: &Path, source: &Source) -> anyhow::Result<Document> {
    let Some(path) = version(dir).and_then(|version| {
        let flags = source.flags().join(" ");
        let key = format!("{}\n{version}\n{flags}", env!("CARGO_PKG_VERSION"));
        file(dir, &key)
    }) else {
        return fresh(dir, source);
    };
    if let Some(document) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|json| Document::from_json(&json).ok())
    {
        eprintln!("{}: analyzed before, at this version", dir.display());
        return Ok(document);
    }
    let document = fresh(dir, source)?;
    if let Err(error) = keep(&path, &document) {
        eprintln!("cannot keep the analysis in {}: {error:#}", path.display());
    }
    Ok(document)
}

/// Analyzes `dir` now, saying how its calls resolved.
fn fresh(dir: &Path, source: &Source) -> anyhow::Result<Document> {
    let label = dir.display().to_string();
    let analysis = crate::ir::analyze_dir(dir, source, None, &label)?;
    eprintln!("{}", analysis.summary(&label));
    Ok(analysis.document)
}

/// What version of its source `dir` holds: the commit, and a hash of any changes on
/// top of it, staged or not, and of the files git does not track. `None` outside git.
fn version(dir: &Path) -> Option<String> {
    let run = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .ok()?;
        output.status.success().then_some(output.stdout)
    };
    let commit = String::from_utf8(run(&["rev-parse", "HEAD"])?).ok()?;
    let changes = run(&["diff", "HEAD", "--binary", "--", "."])?;
    let untracked = run(&[
        "ls-files",
        "--others",
        "--exclude-standard",
        "-z",
        "--",
        ".",
    ])?;
    // Untracked files by name and when they last changed, not by their content.
    let mut stamps = String::new();
    for name in untracked.split(|&b| b == 0).filter(|n| !n.is_empty()) {
        let name = String::from_utf8_lossy(name);
        let modified = std::fs::metadata(dir.join(name.as_ref()))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        let _ = write!(stamps, "{name}\0{modified}\0");
    }
    Some(format!(
        "{} {:016x} {:016x}",
        commit.trim(),
        fnv(&String::from_utf8_lossy(&changes)),
        fnv(&stamps)
    ))
}

/// Where the analysis of `dir` under `key` is kept.
fn file(dir: &Path, key: &str) -> Option<PathBuf> {
    let base = cache_dir(
        std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
        cfg!(target_os = "macos"),
    )?;
    Some(
        base.join("analyses")
            .join(format!("{}{:016x}.json", prefix(dir), fnv(key))),
    )
}

/// What every kept analysis of `dir` is named first: its name, and a hash of where
/// it is.
fn prefix(dir: &Path) -> String {
    let name: String = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    format!("{name}-{:016x}-", fnv(&dir.display().to_string()))
}

fn cache_dir(xdg_cache: Option<PathBuf>, home: Option<PathBuf>, mac: bool) -> Option<PathBuf> {
    match (xdg_cache.filter(|d| d.is_absolute()), home) {
        (Some(cache), _) => Some(cache.join("codeviz")),
        (None, Some(home)) if mac => Some(home.join("Library/Caches/codeviz")),
        (None, Some(home)) => Some(home.join(".cache/codeviz")),
        (None, None) => None,
    }
}

/// Writes `document` to `path`, and drops all but the newest few analyses of the
/// same directory.
fn keep(path: &Path, document: &Document) -> anyhow::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("no directory"))?;
    std::fs::create_dir_all(dir)?;
    std::fs::write(path, document.to_json()?)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let same = name
        .as_deref()
        .and_then(|n| n.get(..n.len().saturating_sub(21)));
    let Some(same) = same else {
        return Ok(());
    };
    let mut kept: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(same))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    kept.sort_by_key(|&(modified, _)| std::cmp::Reverse(modified));
    for (_, old) in kept.into_iter().skip(KEPT) {
        let _ = std::fs::remove_file(old);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_analyses_where_the_system_keeps_caches() {
        let home = Some(PathBuf::from("/home/me"));
        assert_eq!(
            cache_dir(None, home.clone(), true),
            Some(PathBuf::from("/home/me/Library/Caches/codeviz"))
        );
        assert_eq!(
            cache_dir(None, home.clone(), false),
            Some(PathBuf::from("/home/me/.cache/codeviz"))
        );
        assert_eq!(
            cache_dir(Some("/c".into()), home, false),
            Some(PathBuf::from("/c/codeviz"))
        );
    }

    #[test]
    fn names_each_directory_apart() {
        let a = prefix(Path::new("/work/shop/api"));
        let b = prefix(Path::new("/other/api"));
        assert!(a.starts_with("api-") && b.starts_with("api-") && a != b);
        assert_eq!(
            a.len(),
            "api-".len() + 17,
            "the 21 characters keep() strips"
        );
    }
}
