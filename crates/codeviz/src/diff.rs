//! Diffs two revisions of the code in a git repository: each is checked out to a
//! scratch directory, analyzed, and opened in the viewer with the changes drawn over
//! the graph.

use std::{
    fmt::Write as _,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, bail};
use codeviz_core::{Change, Diff, System, align};
use codeviz_ir::Document;

use crate::{ir::Language, view::Display};

#[derive(Debug, clap::Args)]
pub(crate) struct DiffArgs {
    /// What to compare: `REV` against the working tree, `A..B`, or `A...B` for B
    /// against where it branched from A, as a pull request shows it.
    #[arg(default_value = "HEAD")]
    revisions: String,
    /// Directory to analyze, inside the repository.
    #[arg(short = 'C', long, default_value = ".")]
    path: PathBuf,
    /// Source language; detected from the directory when omitted.
    #[arg(short, long, value_enum)]
    lang: Option<Language>,
    /// Include `#[cfg(test)]` code.
    #[arg(long)]
    include_tests: bool,
    /// Print what changed and exit instead of opening the viewer.
    #[arg(long)]
    summary: bool,
    /// Also write the two documents to this directory, as `before.json` and `after.json`.
    #[arg(long, value_name = "DIR")]
    save: Option<PathBuf>,
    #[command(flatten)]
    display: Display,
}

pub(crate) fn run(args: &DiffArgs) -> anyhow::Result<()> {
    let path = args
        .path
        .canonicalize()
        .with_context(|| format!("cannot open {}", args.path.display()))?;
    let top = PathBuf::from(git(&path, &["rev-parse", "--show-toplevel"])?);
    let prefix = git(&path, &["rev-parse", "--show-prefix"])?;
    let (base, head) = revisions(&top, &args.revisions)?;

    let analyze = |root: &Path| analyze_dir(root, args.lang, args.include_tests);
    let old = Checkout::new(&top, &base)?;
    let before = analyze(&old.dir.join(&prefix))?;
    let (after, new) = match &head {
        Some(head) => {
            let new = Checkout::new(&top, head)?;
            (analyze(&new.dir.join(&prefix))?, Some(new))
        }
        None => (analyze(&path)?, None),
    };
    if let Some(dir) = &args.save {
        save(dir, &before, &after)?;
    }

    let name = path.file_name().map_or_else(
        || "repository".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let title = format!(
        "{name}  {} → {}",
        old.label,
        new.as_ref().map_or("working tree", |c| c.label.as_str())
    );
    let new_system = System::new(vec![after.clone()]);
    let (aligned, moved) = align(vec![before.clone()], &new_system);
    let diff = Diff::between(&System::new(aligned), &new_system).with_moves(moved);
    if args.summary {
        return print(&title, &diff);
    }
    if diff.is_empty() {
        eprintln!("{title}: nothing changed");
    }
    crate::view::open(vec![after], vec![before], &args.display, &title)
}

fn analyze_dir(
    root: &Path,
    lang: Option<Language>,
    include_tests: bool,
) -> anyhow::Result<Document> {
    crate::ir::analyze_dir(root, lang, include_tests)
        .with_context(|| format!("cannot analyze {}", root.display()))
}

/// The revision to compare from, and the one to compare to (the working tree if none).
fn revisions(top: &Path, spec: &str) -> anyhow::Result<(String, Option<String>)> {
    let or_head = |rev: &str| if rev.is_empty() { "HEAD" } else { rev }.to_owned();
    if let Some((from, to)) = spec.split_once("...") {
        let (from, to) = (or_head(from), or_head(to));
        let base = git(top, &["merge-base", &from, &to])
            .with_context(|| format!("{from} and {to} share no history"))?;
        return Ok((base, Some(to)));
    }
    if let Some((from, to)) = spec.split_once("..") {
        return Ok((or_head(from), Some(or_head(to))));
    }
    Ok((spec.to_owned(), None))
}

/// One revision of the repository, extracted to a scratch directory that is removed
/// when this is dropped.
struct Checkout {
    dir: PathBuf,
    /// The revision's short hash.
    label: String,
}

impl Checkout {
    fn new(top: &Path, revision: &str) -> anyhow::Result<Self> {
        let label = git(
            top,
            &["rev-parse", "--short", &format!("{revision}^{{commit}}")],
        )
        .with_context(|| format!("{revision} is not a commit"))?;
        let dir = std::env::temp_dir().join(format!("codeviz-{}-{label}", std::process::id()));
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("cannot create {}", dir.display()))?;
        let checkout = Self { dir, label };
        let mut archive = Command::new("git")
            .arg("-C")
            .arg(top)
            .args(["archive", "--format=tar", &checkout.label])
            .stdout(Stdio::piped())
            .spawn()
            .context("cannot run git")?;
        let unpacked = Command::new("tar")
            .arg("-x")
            .arg("-C")
            .arg(&checkout.dir)
            .stdin(archive.stdout.take().context("git archive has no output")?)
            .status()
            .context("cannot run tar")?;
        if !archive.wait()?.success() || !unpacked.success() {
            bail!("cannot extract {revision}");
        }
        Ok(checkout)
    }
}

impl Drop for Checkout {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn git(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .context("cannot run git")?;
    if !output.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn save(dir: &Path, before: &Document, after: &Document) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    for (name, document) in [("before.json", before), ("after.json", after)] {
        let path = dir.join(name);
        std::fs::write(&path, document.to_json()? + "\n")
            .with_context(|| format!("cannot write {}", path.display()))?;
    }
    Ok(())
}

fn print(title: &str, diff: &Diff) -> anyhow::Result<()> {
    let mut out = format!(
        "{title}\ncalls: +{} -{}\n",
        diff.added_calls.len(),
        diff.removed_calls.len()
    );
    let mut units: Vec<_> = diff.units.iter().collect();
    units.sort_by(|a, b| a.0.cmp(b.0));
    for (unit, change) in units {
        let sign = match change {
            Change::Added => '+',
            Change::Removed => '-',
            Change::Modified => '~',
            Change::Moved => '>',
        };
        match diff.moved.get(unit) {
            Some(was) => {
                let _ = writeln!(out, "{sign} {unit}  (was {was})");
            }
            None => {
                let _ = writeln!(out, "{sign} {unit}");
            }
        }
    }
    write!(std::io::stdout().lock(), "{out}")?;
    Ok(())
}
