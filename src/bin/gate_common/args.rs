//! The argument contract every rooted gate shares: `--help`, `--root <dir>`
//! (defaulting to the working directory), any gate-specific switches, and
//! exit code 2 for a usage error. Pulled into each gate with `#[path]`, so
//! the product library never carries dev-only code.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::gate_out;

/// Exit code for a usage or setup error, distinct from a failed verdict (1).
const USAGE_EXIT: u8 = 2;

/// Parses the process arguments into the root to check, setting each
/// `switches` flag that was passed. A gate that takes positional values
/// passes `positionals`: every argument not starting with `--` is appended
/// to it, in order; without it such an argument is a usage error. `Err`
/// carries the exit code to return at once: success after `--help`, 2 after
/// a usage error (already reported on stderr).
pub(crate) fn parse(
    prefix: &str,
    help: &str,
    switches: &mut [(&str, &mut bool)],
    positionals: Option<&mut Vec<String>>,
) -> Result<PathBuf, ExitCode> {
    let root = std::env::current_dir().map_err(|error| {
        eprintln!("{prefix}: cannot determine the project root: {error}");
        ExitCode::from(USAGE_EXIT)
    })?;
    parse_from(
        std::env::args_os().skip(1),
        root,
        (prefix, help),
        switches,
        positionals,
    )
}

fn parse_from(
    mut args: impl Iterator<Item = OsString>,
    mut root: PathBuf,
    (prefix, help): (&str, &str),
    switches: &mut [(&str, &mut bool)],
    mut positionals: Option<&mut Vec<String>>,
) -> Result<PathBuf, ExitCode> {
    while let Some(arg) = args.next() {
        if arg == "--help" {
            return Err(match gate_out::line(help) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => gate_out::failed(prefix, &error),
            });
        }
        if arg == "--root" {
            let Some(value) = args.next() else {
                eprintln!("--root requires a project directory");
                return Err(ExitCode::from(USAGE_EXIT));
            };
            root = PathBuf::from(value);
            continue;
        }
        if let Some(flag) = arg
            .to_str()
            .and_then(|name| switches.iter_mut().find(|(switch, _)| *switch == name))
        {
            *flag.1 = true;
            continue;
        }
        match (arg.into_string(), positionals.as_deref_mut()) {
            (Ok(value), Some(values)) if !value.starts_with("--") => values.push(value),
            (Ok(value), _) => {
                eprintln!("unknown argument: {value}");
                return Err(ExitCode::from(USAGE_EXIT));
            }
            (Err(raw), _) => {
                eprintln!("unknown argument: {}", raw.display());
                return Err(ExitCode::from(USAGE_EXIT));
            }
        }
    }
    Ok(root)
}

#[cfg(test)]
mod gate_args_tests {
    use super::*;
    use slugaudit_test_check::{Check, TestResult};

    fn parse_os(
        args: Vec<OsString>,
        switches: &mut [(&str, &mut bool)],
        positionals: Option<&mut Vec<String>>,
    ) -> Result<PathBuf, ExitCode> {
        parse_from(
            args.into_iter(),
            PathBuf::from("/cwd"),
            ("gate", "help"),
            switches,
            positionals,
        )
    }

    fn parse_list(args: &[&str], switches: &mut [(&str, &mut bool)]) -> Result<PathBuf, ExitCode> {
        parse_os(args.iter().map(OsString::from).collect(), switches, None)
    }

    #[test]
    fn a_gate_with_no_arguments_checks_the_working_directory() -> TestResult {
        let root = parse_list(&[], &mut [])
            .map_err(|code| format!("{code:?}"))
            .check("no arguments is valid")?;

        assert_eq!(root, PathBuf::from("/cwd"));
        Ok(())
    }

    #[test]
    fn root_and_declared_switches_are_parsed_in_any_order() -> TestResult {
        let mut record = false;

        let root = parse_list(
            &["--record", "--root", "/elsewhere"],
            &mut [("--record", &mut record)],
        )
        .map_err(|code| format!("{code:?}"))
        .check("valid arguments")?;

        assert_eq!(root, PathBuf::from("/elsewhere"));
        assert!(record, "the declared switch is set");
        Ok(())
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

    #[test]
    fn positional_values_are_collected_only_when_the_gate_takes_them() -> TestResult {
        let mut values = Vec::new();
        let args = || {
            vec![
                OsString::from("35"),
                OsString::from("--root"),
                OsString::from("/r"),
            ]
        };

        let root = parse_os(args(), &mut [], Some(&mut values))
            .map_err(|code| format!("{code:?}"))
            .check("positional values are accepted")?;
        let refused = parse_os(args(), &mut [], None);

        assert_eq!(root, PathBuf::from("/r"));
        assert_eq!(values, ["35"]);
        assert_eq!(refused, Err(ExitCode::from(USAGE_EXIT)));
        Ok(())
    }

    #[test]
    fn an_unknown_double_dash_flag_is_refused_even_when_positionals_are_taken() {
        let mut values = Vec::new();

        let outcome = parse_os(vec![OsString::from("--bogus")], &mut [], Some(&mut values));

        assert_eq!(outcome, Err(ExitCode::from(USAGE_EXIT)));
        assert!(values.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_root_that_is_not_utf8_is_kept_byte_for_byte() -> TestResult {
        use std::os::unix::ffi::OsStringExt;
        let raw = OsString::from_vec(vec![b'/', b'r', 0xff, b'x']);

        let root = parse_os(vec![OsString::from("--root"), raw.clone()], &mut [], None)
            .map_err(|code| format!("{code:?}"))
            .check("a non-UTF-8 root parses")?;

        assert_eq!(root.as_os_str(), raw.as_os_str());
        Ok(())
    }
}
