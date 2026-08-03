use super::*;

#[test]
fn no_arguments_means_serve() {
    assert_eq!(parse_args(std::iter::empty()), Command::Serve);
}

#[test]
fn explicit_serve() {
    assert_eq!(
        parse_args(vec!["serve".to_owned()].into_iter()),
        Command::Serve
    );
}

#[test]
fn enable_defaults_to_current_directory() {
    assert_eq!(
        parse_args(vec!["enable".to_owned()].into_iter()),
        Command::Enable {
            path: PathBuf::from(".")
        }
    );
}

#[test]
fn enable_with_an_explicit_path() {
    assert_eq!(
        parse_args(vec!["enable".to_owned(), "some/project".to_owned()].into_iter()),
        Command::Enable {
            path: PathBuf::from("some/project")
        }
    );
}

#[test]
fn disable_parses_the_yes_flag_in_either_form() {
    assert_eq!(
        parse_args(vec!["disable".to_owned(), "-y".to_owned()].into_iter()),
        Command::Disable {
            path: PathBuf::from("."),
            assume_yes: true
        }
    );
    assert_eq!(
        parse_args(vec!["disable".to_owned(), "a/b".to_owned(), "--yes".to_owned()].into_iter()),
        Command::Disable {
            path: PathBuf::from("a/b"),
            assume_yes: true
        }
    );
}

#[test]
fn unrecognized_input_shows_help_rather_than_silently_serving() {
    assert_eq!(
        parse_args(vec!["--bogus".to_owned()].into_iter()),
        Command::Help
    );
    assert_eq!(
        parse_args(vec!["help".to_owned()].into_iter()),
        Command::Help
    );
}

#[test]
fn enable_creates_the_marker_and_runs_a_real_initial_import() {
    let project = tempfile::tempdir().expect("project dir");
    std::fs::write(project.path().join("lib.rs"), b"pub fn a() {}\n").expect("write fixture file");

    run_enable(project.path()).expect("enable succeeds");

    assert!(project.path().join(".planning").join("slugaudit").is_dir());
    let db_path = project
        .path()
        .join(".planning")
        .join("slugaudit")
        .join("project.db");
    let connection =
        crate::store::open_read_only(&db_path).expect("open the database enable created");
    let file_count: i64 = connection
        .query_row("SELECT count(*) FROM files", [], |row| row.get(0))
        .expect("query files");
    assert_eq!(
        file_count, 1,
        "enable must run a real import, not just create the marker"
    );
}

#[test]
fn disable_with_assume_yes_skips_the_prompt_and_removes_everything() {
    let project = tempfile::tempdir().expect("project dir");
    run_enable(project.path()).expect("enable first");
    assert!(project.path().join(".planning").join("slugaudit").is_dir());

    run_disable(project.path(), true).expect("disable succeeds");
    assert!(!project.path().join(".planning").join("slugaudit").exists());
}

#[test]
fn disabling_an_already_inactive_project_does_not_prompt_or_error() {
    let project = tempfile::tempdir().expect("project dir");
    // No stdin input provided at all — if this path tried to prompt, the
    // empty reader would still just read zero bytes rather than hang, but
    // asserting Ok(()) here is really asserting the early return happens
    // before any prompt is attempted.
    let result = disable_with_input(project.path(), false, std::io::Cursor::new(Vec::new()));
    assert!(result.is_ok());
}

#[test]
fn disable_without_assume_yes_respects_a_no_answer() {
    let project = tempfile::tempdir().expect("project dir");
    run_enable(project.path()).expect("enable first");

    disable_with_input(project.path(), false, std::io::Cursor::new(b"n\n".to_vec()))
        .expect("declining must not be an error");
    assert!(
        project.path().join(".planning").join("slugaudit").is_dir(),
        "declining the prompt must leave the project enabled"
    );
}

#[test]
fn disable_without_assume_yes_respects_a_yes_answer() {
    let project = tempfile::tempdir().expect("project dir");
    run_enable(project.path()).expect("enable first");

    disable_with_input(project.path(), false, std::io::Cursor::new(b"y\n".to_vec()))
        .expect("confirmed disable");
    assert!(!project.path().join(".planning").join("slugaudit").exists());
}
