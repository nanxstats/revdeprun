use std::fs;

use tempfile::tempdir;
use xshell::{Shell, cmd};

#[test]
fn bundle_prints_summary_to_stderr_without_a_terminal() {
    let tmp = tempdir().expect("tempdir");
    let pkg = tmp.path().join("pkg");
    fs::create_dir_all(pkg.join("alpha.Rcheck")).expect("check directory");
    fs::write(pkg.join("DESCRIPTION"), "Package: pkg\nVersion: 1.0\n").expect("DESCRIPTION");
    fs::write(pkg.join("alpha.Rcheck/00check.log"), "Status: 1 WARNING\n").expect("check log");

    let shell = Shell::new().expect("shell");
    let bin = env!("CARGO_BIN_EXE_revdeprun");
    // Capturing both streams gives the CLI no terminal, as with noninteractive SSH.
    let result = cmd!(shell, "{bin} bundle {pkg}")
        .output()
        .expect("bundle command");
    let output = tmp.path().join("pkg-revdep.tar.zst");
    assert!(output.is_file());
    assert!(result.stdout.is_empty(), "bundle must leave stdout unused");

    let stderr = String::from_utf8(result.stderr).expect("UTF-8 stderr");
    assert!(stderr.contains("Reverse dependency check results bundled successfully."));
    assert!(
        stderr.contains(
            &output
                .canonicalize()
                .expect("absolute output")
                .display()
                .to_string()
        )
    );
    assert!(stderr.contains("\n  scp "));
    assert!(stderr.contains("\n  tar -xf pkg-revdep.tar.zst"));
}
