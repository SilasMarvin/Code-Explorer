//! Diffs two revisions of the code in a git repository: each is checked out to a
//! scratch directory, analyzed, and opened in the viewer with the changes drawn over
//! the graph.

use std::{
    fmt::Write as _,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

use anyhow::{Context, bail};
use codeviz_core::{Change, Diff, Review, System, align};
use codeviz_ir::{Document, UnitId};

use crate::{ir::Source, view::Display};

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

    // The checkouts have no build of their own: rust-analyzer builds their build
    // scripts and proc macros where the working tree's are, not from scratch.
    let target_dir = crate::ir::target_dir(&path);
    let analyze = |root: &Path, step: &str, label: &str| {
        progress(step, &format!("analyzing {label}"));
        let started = Instant::now();
        let document = crate::ir::analyze_dir(root, &args.source, target_dir.as_deref())
            .with_context(|| format!("cannot analyze {}", root.display()))?;
        progress(
            step,
            &format!(
                "{label}: {} units in {:.1}s",
                document.units.len(),
                started.elapsed().as_secs_f32()
            ),
        );
        anyhow::Ok(document)
    };
    let old = Checkout::new(&top, &base)?;
    let before = analyze(&old.dir.join(&prefix), "1/2", &old.label)?;
    let (after, new) = match &head {
        Some(head) => {
            let new = Checkout::new(&top, head)?;
            let label = new.label.clone();
            (analyze(&new.dir.join(&prefix), "2/2", &label)?, Some(new))
        }
        None => (analyze(&path, "2/2", "the working tree")?, None),
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
    let old_system = System::new(aligned);
    let diff = Diff::between(&old_system, &new_system).with_moves(moved);
    if args.summary {
        let review = Review::new(&diff, &old_system, &new_system);
        let summary = summary(&title, &diff, &review, &old_system, &new_system);
        write!(std::io::stdout().lock(), "{summary}")?;
        return Ok(());
    }
    if diff.is_empty() {
        eprintln!("{title}: nothing changed");
    }
    crate::view::open(vec![after], vec![before], &args.display, &title)
}

/// One line on how far the diff has got, on stderr: each analysis can take a while.
fn progress(step: &str, what: &str) {
    eprintln!("codeviz diff [{step}]: {what}");
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

/// What changed, as text: the totals, the entry points the changes reach, the changed
/// functions in the order to read them, and what changed besides functions.
fn summary(title: &str, diff: &Diff, review: &Review, before: &System, after: &System) -> String {
    let count = |change| {
        diff.changed(change)
            .filter(|id| review.order.contains(id))
            .count()
    };
    let mut out = format!(
        "{title}\n{} functions changed: +{} added, ~{} rewritten, -{} removed, >{} moved; \
         calls +{} -{}\n",
        review.order.len(),
        count(Change::Added),
        count(Change::Modified),
        count(Change::Removed),
        count(Change::Moved),
        diff.added_calls.len(),
        diff.removed_calls.len()
    );
    let affected = review.affected();
    let starts = if review.roots {
        "functions nothing calls"
    } else {
        "entry points"
    };
    let _ = writeln!(
        out,
        "reaches {} of {} {starts}",
        affected.len(),
        review.starts.len()
    );
    for &start in &affected {
        let _ = writeln!(out, "  {}", review.starts[start].label);
    }
    let _ = writeln!(
        out,
        "\nin reading order, each change after the changes it calls:"
    );
    let path = |id: &UnitId| {
        let id = id.as_str();
        id.split_once(':').map_or(id, |(_, path)| path).to_owned()
    };
    for (index, unit) in review.order.iter().enumerate() {
        let Some(change) = diff.change(unit) else {
            continue;
        };
        let mut line = format!("{:>4} {} {}", index + 1, sign(change), path(unit));
        if let Some(was) = diff.moved.get(unit) {
            let _ = write!(line, "  (was {})", path(was));
        }
        let reached = review.reached_from(unit).len();
        if reached > 0 {
            let _ = write!(line, "  · affects {reached}");
        }
        let _ = writeln!(out, "{line}");
    }
    let mut others: Vec<(&UnitId, Change)> = diff
        .units
        .iter()
        .filter(|(id, _)| !review.order.contains(id))
        .map(|(id, &change)| (id, change))
        .collect();
    others.sort();
    if !others.is_empty() {
        let _ = writeln!(out, "\nalso changed, not drawn:");
    }
    for (unit, change) in others {
        let kind = after
            .unit(unit)
            .or_else(|| before.unit(unit))
            .map_or("", |u| u.kind.as_str());
        let _ = writeln!(out, "     {} {kind} {}", sign(change), path(unit));
    }
    out
}

fn sign(change: Change) -> char {
    match change {
        Change::Added => '+',
        Change::Removed => '-',
        Change::Modified => '~',
        Change::Moved => '>',
    }
}
