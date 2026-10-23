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

use crate::{
    ir::Source,
    view::{Display, Origin},
};

#[derive(Debug, clap::Args)]
pub(crate) struct DiffArgs {
    /// What to compare: `REV` against the working tree, `A..B`, or `A...B` for B
    /// against where it branched from A, as a pull request shows it.
    #[arg(default_value = "HEAD")]
    revisions: String,
    /// Directory to analyze, inside the repository.
    #[arg(short = 'C', long, default_value = ".")]
    path: PathBuf,
    #[command(flatten)]
    source: Source,
    /// Print what changed and exit instead of opening the viewer.
    #[arg(long)]
    summary: bool,
    /// Also write the two documents to this directory, as `before.json` and
    /// `after.json`, with a copy of the source each checked-out revision points at.
    #[arg(long, value_name = "DIR")]
    save: Option<PathBuf>,
    #[command(flatten)]
    pub(crate) display: Display,
}

impl DiffArgs {
    /// What names this diff, for keeping its last view, and a name for it.
    pub(crate) fn state_key(&self) -> (String, String) {
        let path = self
            .path
            .canonicalize()
            .unwrap_or_else(|_| self.path.clone());
        let name = path.file_name().map_or_else(
            || "repository".to_owned(),
            |n| n.to_string_lossy().into_owned(),
        );
        (
            format!("diff\n{}\n{}", path.display(), self.revisions),
            format!("{name} {}", self.revisions),
        )
    }

    /// The command that runs this diff again, ahead of the view's own flags.
    fn command(&self) -> Vec<String> {
        let mut command = vec![
            "codeviz".to_owned(),
            "diff".to_owned(),
            self.revisions.clone(),
        ];
        if self.path != Path::new(".") {
            command.extend(["-C".to_owned(), self.path.display().to_string()]);
        }
        command.extend(self.source.flags());
        command
    }
}

pub(crate) fn run(args: &DiffArgs) -> anyhow::Result<()> {
    let path = args
        .path
        .canonicalize()
        .with_context(|| format!("cannot open {}", args.path.display()))?;
    let top = PathBuf::from(git(&path, &["rev-parse", "--show-toplevel"])?);
    let prefix = git(&path, &["rev-parse", "--show-prefix"])?;
    let (base, head) = revisions(&top, &args.revisions)?;

    // The checkouts have no build of their own: rust-analyzer builds their build
    // scripts and proc macros where the working tree's are, not from scratch.
    let target_dir = crate::ir::target_dir(&path);
    let analyze = |root: &Path| {
        crate::ir::analyze_dir(root, &args.source, target_dir.as_deref())
            .with_context(|| format!("cannot analyze {}", root.display()))
    };
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
        save(dir, &before, &after, head.is_some())?;
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
    let origin = Origin {
        title,
        command: args.command(),
        key: args.state_key(),
        // The newer revision's functions open in the editor from the working tree,
        // not from a checkout gone once this exits.
        edit_root: new
            .as_ref()
            .map(|new| (new.dir.join(&prefix), path.clone())),
    };
    crate::view::open(vec![after], vec![before], &args.display, origin)
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

/// Writes both documents to `dir`. A side analyzed in a scratch checkout takes a copy
/// of the files it points at, in `before/` or `after/` beside it, since the checkout
/// goes when this exits; a side analyzed in the working tree points there.
fn save(
    dir: &Path,
    before: &Document,
    after: &Document,
    after_checked_out: bool,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let dir = dir.canonicalize()?;
    for (name, document, checked_out) in [
        ("before", before, true),
        ("after", after, after_checked_out),
    ] {
        let mut document = document.clone();
        if checked_out {
            keep_source(&mut document, &dir.join(name))?;
        }
        let path = dir.join(format!("{name}.json"));
        std::fs::write(&path, document.to_json()? + "\n")
            .with_context(|| format!("cannot write {}", path.display()))?;
    }
    Ok(())
}

/// Copies the files `document` points at under its root to `to`, and roots it there.
fn keep_source(document: &mut Document, to: &Path) -> anyhow::Result<()> {
    let Some(from) = document
        .root
        .as_deref()
        .and_then(|r| r.strip_prefix("file://"))
        .map(PathBuf::from)
    else {
        return Ok(());
    };
    for file in crate::ir::relative_files(document) {
        let inside = Path::new(&file)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)));
        if !inside {
            continue;
        }
        let target = to.join(&file);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        std::fs::copy(from.join(&file), &target)
            .with_context(|| format!("cannot copy {file} to {}", to.display()))?;
    }
    document.root = Some(crate::ir::file_uri(to));
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
