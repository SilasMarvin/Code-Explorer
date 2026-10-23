//! Keeps a live view current. A thread polls what the view opened, IR documents and
//! directories of source, and once a change has settled reads what changed again and
//! sends the viewer the new documents. Directories are analyzed from the syntax alone,
//! which keeps up with every save. A version in which a file stopped parsing is held
//! back, so the viewer keeps the last good one and says why.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, Sender, channel},
    time::{Duration, Instant, SystemTime},
};

use anyhow::Context;
use codeviz_ir::Document;
use codeviz_viewer::Update;
use walkdir::WalkDir;

use crate::ir::Source;

/// How often to look for changes, at the least.
const POLL: Duration = Duration::from_millis(250);
/// How long a change must hold still before it is read: an editor may save in steps.
const SETTLE: Duration = Duration::from_millis(120);
/// Looking for changes takes at most this share of the time.
const LOOKING: u32 = 10;

/// Something a view opened: an IR document, or a directory of source to analyze.
#[derive(Debug, Clone)]
pub(crate) enum Input {
    Document(PathBuf),
    Source(PathBuf),
}

/// An input as read: its document, and the files in it that did not wholly parse, by
/// path from the input with the line each stops at.
#[derive(Debug)]
pub(crate) struct Read {
    pub(crate) document: Document,
    pub(crate) unparsed: Vec<(PathBuf, usize)>,
}

impl Input {
    pub(crate) fn new(path: &Path) -> Self {
        if path.is_dir() {
            Self::Source(path.to_path_buf())
        } else {
            Self::Document(path.to_path_buf())
        }
    }

    pub(crate) fn read(&self, source: &Source) -> anyhow::Result<Read> {
        match self {
            Self::Document(path) => Ok(Read {
                document: crate::ir::read(path)?,
                unparsed: Vec::new(),
            }),
            Self::Source(dir) => {
                let root = dir
                    .canonicalize()
                    .with_context(|| format!("cannot open {}", dir.display()))?;
                let analyzed = crate::ir::analyze_source(&root, source, None)
                    .with_context(|| format!("cannot analyze {}", dir.display()))?;
                Ok(Read {
                    document: analyzed.document,
                    unparsed: analyzed.unparsed,
                })
            }
        }
    }

    /// Every file the input reads, with when it was last written and its size: what a
    /// change shows up in.
    fn stamp(&self) -> Stamp {
        match self {
            Self::Document(path) => vec![stamp(path)],
            Self::Source(dir) => WalkDir::new(dir)
                .sort_by_file_name()
                .into_iter()
                .filter_entry(|entry| entry.depth() == 0 || !skipped(entry.file_name()))
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_file() && read_by_analysis(entry.path()))
                .map(|entry| stamp(entry.path()))
                .collect(),
        }
    }
}

type Stamp = Vec<(PathBuf, Option<SystemTime>, u64)>;

fn stamp(path: &Path) -> (PathBuf, Option<SystemTime>, u64) {
    let metadata = std::fs::metadata(path).ok();
    (
        path.to_path_buf(),
        metadata.as_ref().and_then(|m| m.modified().ok()),
        metadata.map_or(0, |m| m.len()),
    )
}

/// Directories no analysis reads: hidden ones, build output, installed packages.
fn skipped(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy();
    name.starts_with('.') || ["target", "node_modules", "__pycache__", "venv"].contains(&&*name)
}

/// Files an analysis reads: Rust and Python source, and Cargo manifests.
fn read_by_analysis(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext == "rs" || ext == "py")
        || path.file_name().is_some_and(|name| name == "Cargo.toml")
}

/// Watches `inputs`, read as `opened` when the view opened, and sends what they are
/// each time they change. Their files that did not parse then are let be; a file that
/// stops parsing holds the update back until it parses again.
pub(crate) fn spawn(inputs: Vec<Input>, source: Source, opened: Vec<Read>) -> Receiver<Update> {
    let (send, updates) = channel();
    let files: usize = inputs.iter().map(|input| input.stamp().len()).sum();
    eprintln!("codeviz: watching {files} files for changes");
    std::thread::spawn(move || watch(&inputs, &source, opened, &send));
    updates
}

fn watch(inputs: &[Input], source: &Source, opened: Vec<Read>, send: &Sender<Update>) {
    let mut stamps: Vec<Stamp> = inputs.iter().map(Input::stamp).collect();
    let (mut documents, mut tolerated): (Vec<Document>, Vec<BTreeSet<PathBuf>>) = opened
        .into_iter()
        .map(|read| {
            (
                read.document,
                read.unparsed.into_iter().map(|u| u.0).collect(),
            )
        })
        .unzip();
    // Inputs that changed but have not been read well since.
    let mut pending = BTreeSet::new();
    loop {
        let started = Instant::now();
        let now: Vec<Stamp> = inputs.iter().map(Input::stamp).collect();
        if now == stamps {
            std::thread::sleep(POLL.max(started.elapsed() * LOOKING));
            continue;
        }
        std::thread::sleep(SETTLE);
        if inputs.iter().map(Input::stamp).ne(now.iter().cloned()) {
            continue;
        }
        pending.extend((0..inputs.len()).filter(|&i| now[i] != stamps[i]));
        stamps = now;
        if send.send(Update::Reading).is_err() {
            return;
        }
        let update = match read_pending(inputs, source, &pending, &tolerated) {
            Ok(read) => {
                for (index, read) in read {
                    documents[index] = read.document;
                    tolerated[index] = read.unparsed.into_iter().map(|u| u.0).collect();
                }
                pending.clear();
                Update::Documents(documents.clone())
            }
            Err(why) => Update::Failed(why),
        };
        if send.send(update).is_err() {
            return;
        }
    }
}

/// Reads every input in `pending`, or says why one cannot be taken: it failed to read,
/// or a file in it stopped parsing that parsed before.
fn read_pending(
    inputs: &[Input],
    source: &Source,
    pending: &BTreeSet<usize>,
    tolerated: &[BTreeSet<PathBuf>],
) -> Result<Vec<(usize, Read)>, String> {
    pending
        .iter()
        .map(|&index| {
            let read = inputs[index].read(source).map_err(|e| format!("{e:#}"))?;
            if let Some((path, line)) = read
                .unparsed
                .iter()
                .find(|(path, _)| !tolerated[index].contains(path))
            {
                return Err(format!("{}:{line} does not parse", path.display()));
            }
            Ok((index, read))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_directory_is_stamped_by_the_files_analysis_reads() {
        let root = std::env::temp_dir().join(format!("codeviz-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (path, text) in [
            ("Cargo.toml", "[package]"),
            ("src/lib.rs", "fn a() {}"),
            ("src/notes.md", "notes"),
            ("target/debug/build.rs", "fn b() {}"),
            (".git/x.rs", ""),
            ("py/app.py", "def c(): pass"),
        ] {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap_or(&root)).unwrap_or_default();
            std::fs::write(path, text).unwrap_or_default();
        }
        let stamped: Vec<PathBuf> = Input::new(&root)
            .stamp()
            .into_iter()
            .map(|(path, _, _)| path.strip_prefix(&root).unwrap_or(&path).to_path_buf())
            .collect();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(
            stamped,
            ["Cargo.toml", "py/app.py", "src/lib.rs"].map(PathBuf::from)
        );
    }
}
