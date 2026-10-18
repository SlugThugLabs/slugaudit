//! The argument contract every rooted gate shares: `--help`, `--root <dir>`
//! (defaulting to the working directory), any gate-specific switches, and
//! exit code 2 for a usage error. Pulled into each gate with `#[path]`, so
//! the product library never carries dev-only code (DECISIONS 2026-10-16,
//! "Gate-bin argument parsing shared").

use std::path::PathBuf;
use std::process::ExitCode;

/// Exit code for a usage or setup error, distinct from a failed verdict (1).
const USAGE_EXIT: u8 = 2;

/// Parses the process arguments into the root to check, setting each
/// `switches` flag that was passed. `Err` carries the exit code to return
/// at once: success after `--help`, 2 after a usage error (already reported
/// on stderr).
pub(crate) fn parse(
    prefix: &str,
    help: &str,
    switches: &mut [(&str, &mut bool)],
) -> Result<PathBuf, ExitCode> {
    let root = std::env::current_dir().map_err(|error| {
        eprintln!("{prefix}: cannot determine the project root: {error}");
        ExitCode::from(USAGE_EXIT)
    })?;
    let args = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned());
    parse_from(args, root, help, switches)
}

fn parse_from(
    mut args: impl Iterator<Item = String>,
    mut root: PathBuf,
    help: &str,
    switches: &mut [(&str, &mut bool)],
) -> Result<PathBuf, ExitCode> {
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" => {
                println!("{help}");
                return Err(ExitCode::SUCCESS);
            }
            "--root" => {
                let Some(value) = args.next() else {
                    eprintln!("--root requires a project directory");
                    return Err(ExitCode::from(USAGE_EXIT));
                };
                root = PathBuf::from(value);
            }
            other => match switches.iter_mut().find(|(name, _)| *name == other) {
                Some((_, flag)) => **flag = true,
                None => {
                    eprintln!("unknown argument: {other}");
                    return Err(ExitCode::from(USAGE_EXIT));
                }
            },
        }
    }
    Ok(root)
}

#[cfg(test)]
mod gate_args_tests {
    use super::*;

    fn parse_list(args: &[&str], switches: &mut [(&str, &mut bool)]) -> Result<PathBuf, ExitCode> {
        let args = args.iter().map(|arg| (*arg).to_owned());
        parse_from(args, PathBuf::from("/cwd"), "help", switches)
    }

    #[test]
    fn a_gate_with_no_arguments_checks_the_working_directory() {
        let root = parse_list(&[], &mut []).expect("no arguments is valid");

        assert_eq!(root, PathBuf::from("/cwd"));
    }

    #[test]
    fn root_and_declared_switches_are_parsed_in_any_order() {
        let mut record = false;

        let root = parse_list(
            &["--record", "--root", "/elsewhere"],
            &mut [("--record", &mut record)],
        )
        .expect("valid arguments");

        assert_eq!(root, PathBuf::from("/elsewhere"));
        assert!(record, "the declared switch is set");
    }

    #[test]
    fn usage_errors_and_help_stop_the_gate_before_it_runs() {
        let cases: &[(&[&str], ExitCode)] = &[
            (&["--root"], ExitCode::from(USAGE_EXIT)),
            (&["--record"], ExitCode::from(USAGE_EXIT)),
            (&["--help"], ExitCode::SUCCESS),
        ];

        for (args, expected) in cases {
            let outcome = parse_list(args, &mut []);

            assert_eq!(
                outcome,
                Err(*expected),
                "{args:?} must stop with its own exit code"
            );
        }
    }
}
