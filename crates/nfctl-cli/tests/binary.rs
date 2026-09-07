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
        .stdout(predicates::str::contains("complete -c nfctl"));
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
