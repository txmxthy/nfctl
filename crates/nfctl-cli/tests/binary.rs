#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The binary's contract: exit codes and offline commands. No cluster needed.

use assert_cmd::Command;

#[test]
fn unimplemented_error_exits_4() {
    // No stub commands remain; the exit-code contract is kept by the error type.
    assert_eq!(nfctl_core::Error::Unimplemented("x").exit_code(), 4);
    assert!(
        nfctl_core::Error::Unimplemented("x")
            .to_string()
            .contains("roadmap")
    );
}

#[test]
fn completions_do_not_need_a_cluster() {
    Command::cargo_bin("nfctl")
        .unwrap()
        .args(["completions", "fish"])
        .assert()
        .success()
        .stdout(predicates::str::contains("COMPLETE=fish nfctl"));
}

/// The shell calls back with the line after `--`; names come from the fixture.
#[test]
fn dynamic_completion_offers_fixture_resources() {
    let complete = |args: &[&str]| -> String {
        let out = Command::cargo_bin("nfctl")
            .unwrap()
            .env("COMPLETE", "fish")
            .env("NFCTL_FIXTURE", "../../examples/fixtures/demo.yaml")
            .arg("--")
            .args(args)
            .assert()
            .success();
        // An empty current token also lists flags; only names matter here.
        String::from_utf8_lossy(&out.get_output().stdout)
            .lines()
            .filter(|l| !l.starts_with('-'))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        complete(&["nfctl", "get", "f"]).trim(),
        "fanout\tRunning · default"
    );
    let vertices = complete(&["nfctl", "logs", "fanout", "e"]);
    assert_eq!(vertices.trim(), "even-or-odd\neven-sink");
    assert!(complete(&["nfctl", "mvtx", "get", ""]).starts_with("mono\t"));
    assert!(complete(&["nfctl", "isb", "inspect", ""]).starts_with("default\t"));
    // A namespace on the line narrows the candidates.
    assert_eq!(complete(&["nfctl", "-n", "nowhere", "get", ""]).trim(), "");
}

#[test]
fn map_prints_the_whole_surface_offline() {
    let out = Command::cargo_bin("nfctl")
        .unwrap()
        .arg("map")
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    insta::assert_snapshot!("map", text);
    Command::cargo_bin("nfctl")
        .unwrap()
        .args(["map", "-o", "json"])
        .assert()
        .success()
        .stdout(predicates::str::contains("\"children\""));
}

#[test]
fn help_lists_every_command() {
    let out = Command::cargo_bin("nfctl")
        .unwrap()
        .arg("--help")
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    for cmd in [
        "map",
        "ls",
        "get",
        "dag",
        "logs",
        "top",
        "pause",
        "resume",
        "recycle",
        "apply",
        "tui",
        "completions",
    ] {
        assert!(
            text.contains(&format!("\n  {cmd}")),
            "missing `{cmd}` in help:\n{text}"
        );
    }
}
