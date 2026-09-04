#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The binary's contract: exit codes and offline commands. No cluster needed.

use assert_cmd::Command;

#[test]
fn stub_exits_4_and_points_at_roadmap() {
    Command::cargo_bin("nfctl")
        .unwrap()
        .args(["tui"])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("roadmap"));
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
fn help_lists_every_command() {
    let out = Command::cargo_bin("nfctl")
        .unwrap()
        .arg("--help")
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    for cmd in [
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
