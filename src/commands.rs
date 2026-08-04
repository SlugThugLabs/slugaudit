//! Human-facing project lifecycle commands.
#![allow(clippy::print_stdout)]

use crate::project::{self, ProjectRoot};
use crate::{parse, store, sync};
use std::io::{BufRead, Write as _};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    #[error(transparent)]
    Root(#[from] project::RootError),
    #[error(transparent)]
    Activation(#[from] project::ActivationError),
    #[error(transparent)]
    Store(#[from] store::StoreError),
    #[error(transparent)]
    Publish(#[from] sync::PublishError),
    #[error("failed to read from the terminal: {0}")]
    Io(#[from] std::io::Error),
}

pub fn run_enable(path: &Path) -> Result<(), CliError> {
    let root = ProjectRoot::resolve(path)?;
    project::enable(&root)?;
    println!("SlugAudit enabled for {}", root.as_path().display());
    println!("Running initial import...");
    let mut connection = store::open_read_write(&project::database_path(&root))?;
    let report = sync::publish(&mut connection, root.as_path(), parse::PACK_VERSION)?;
    println!(
        "Initial import complete: {} file(s) added, {} unchanged (revision {})",
        report.added, report.unchanged, report.revision_id
    );
    Ok(())
}

pub fn run_disable(path: &Path, assume_yes: bool) -> Result<(), CliError> {
    disable_with_input(path, assume_yes, std::io::stdin().lock())
}

fn disable_with_input(path: &Path, assume_yes: bool, input: impl BufRead) -> Result<(), CliError> {
    let root = ProjectRoot::resolve(path)?;
    let activation = project::activation_dir(&root);
    if !activation.exists() {
        println!("SlugAudit is not enabled for {}", root.as_path().display());
        return Ok(());
    }
    if !assume_yes {
        let prompt = format!(
            "This will permanently delete SlugAudit's disposable index for {} \
             (database, findings, evidence). Continue? [y/N] ",
            activation.display()
        );
        if !confirm(&prompt, input)? {
            println!("Cancelled.");
            return Ok(());
        }
    }
    project::disable(&root)?;
    println!("SlugAudit disabled for {}", root.as_path().display());
    Ok(())
}

fn confirm(prompt: &str, mut input: impl BufRead) -> Result<bool, std::io::Error> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    input.read_line(&mut line)?;
    Ok(line.trim().eq_ignore_ascii_case("y"))
}
