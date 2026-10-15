//! Opens IR documents in the Bevy viewer.

use std::path::PathBuf;

use anyhow::bail;
use codeviz_core::{JourneyOptions, System};
use codeviz_viewer::Viewer;

#[derive(Debug, clap::Args)]
pub(crate) struct ViewArgs {
    /// IR documents to load; later documents override earlier ones.
    #[arg(required = true)]
    documents: Vec<PathBuf>,
    /// Entry label (`POST /users`) or unit id suffix to start from.
    #[arg(short, long)]
    entry: Option<String>,
    /// Include calls nothing resolved.
    #[arg(long)]
    unresolved: bool,
    /// Render once, save a PNG screenshot here, and exit.
    #[arg(long, value_name = "PATH")]
    screenshot: Option<PathBuf>,
}

pub(crate) fn run(args: &ViewArgs) -> anyhow::Result<()> {
    let documents = args
        .documents
        .iter()
        .map(crate::ir::read)
        .collect::<anyhow::Result<Vec<_>>>()?;
    let system = System::new(documents);
    let root = match &args.entry {
        Some(query) => match system.find(query).as_slice() {
            [unit] => Some(unit.id.clone()),
            [] => bail!("nothing matches `{query}`"),
            _ => bail!("`{query}` is ambiguous; run `codeviz journey` to list candidates"),
        },
        None => None,
    };
    let title = format!(
        "codeviz — {}",
        args.documents
            .iter()
            .filter_map(|p| p.file_stem())
            .map(|s| s.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" + ")
    );

    let mut viewer = Viewer::new(system)
        .with_title(title)
        .with_options(JourneyOptions {
            include_unresolved: args.unresolved,
            ..JourneyOptions::default()
        });
    if let Some(root) = root {
        viewer = viewer.with_root(root);
    }
    if let Some(path) = &args.screenshot {
        viewer = viewer.with_screenshot(path);
    }
    if viewer.run().is_error() {
        bail!("viewer exited with an error");
    }
    Ok(())
}
