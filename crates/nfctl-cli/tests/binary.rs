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

/// A Tab press must never put anything on the terminal we did not choose to
/// write, even when the context's credential plugin fails. Nothing here can
/// reach a cluster: the kubeconfig is empty and the context does not exist.
#[test]
fn dynamic_completion_is_silent_on_a_broken_context() {
    let cache = std::env::temp_dir().join("nfctl-completion-silence-cache");
    let kubeconfig = std::env::temp_dir().join("nfctl-completion-silence-kubeconfig");
    let out = Command::cargo_bin("nfctl")
        .unwrap()
        .env("COMPLETE", "fish")
        .env("KUBECONFIG", &kubeconfig)
        .env("XDG_CACHE_HOME", &cache)
        .env_remove("NFCTL_FIXTURE")
        .env_remove("NFCTL_CONTEXT")
        .args(["--", "nfctl", "--context", "does-not-exist-xyz", "get", "f"])
        .assert()
        .success();
    assert_eq!(
        String::from_utf8_lossy(&out.get_output().stderr),
        "",
        "the completion path wrote to the terminal"
    );
    // No cluster and no cache: the right answer is no candidates at all.
    assert_eq!(String::from_utf8_lossy(&out.get_output().stdout).trim(), "");
    std::fs::remove_dir_all(&cache).ok();
}

/// A kubeconfig whose only context authenticates through an exec credential
/// plugin that fails noisily, the way a cloud plugin does when it cannot
/// refresh. The server is unroutable, so nothing leaves the machine.
#[cfg(unix)]
fn noisy_plugin_kubeconfig(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("kubeconfig.yaml");
    std::fs::write(
        &path,
        "apiVersion: v1\n\
         kind: Config\n\
         current-context: noisy\n\
         clusters:\n\
         - name: noisy\n\
           cluster: {server: 'https://127.0.0.1:1'}\n\
         contexts:\n\
         - name: noisy\n\
           context: {cluster: noisy, user: noisy}\n\
         users:\n\
         - name: noisy\n\
           user:\n\
             exec:\n\
               apiVersion: client.authentication.k8s.io/v1beta1\n\
               command: sh\n\
               args: ['-c', 'echo PLUGIN-NOISE >&2; exit 1']\n",
    )
    .unwrap();
    path
}

/// The bug: the credential plugin is a child process that inherits our stderr,
/// so its message lands on the prompt line without passing through our code.
/// Completing must swallow it; a normal run must still show it.
#[cfg(unix)]
#[test]
fn a_failing_credential_plugin_is_silent_only_while_completing() {
    let dir = std::env::temp_dir().join("nfctl-noisy-plugin");
    std::fs::create_dir_all(&dir).unwrap();
    let kubeconfig = noisy_plugin_kubeconfig(&dir);
    let cache = dir.join("cache");

    let completing = Command::cargo_bin("nfctl")
        .unwrap()
        .env("COMPLETE", "fish")
        .env("KUBECONFIG", &kubeconfig)
        .env("XDG_CACHE_HOME", &cache)
        .env_remove("NFCTL_FIXTURE")
        .env_remove("NFCTL_CONTEXT")
        .args(["--", "nfctl", "--context", "noisy", "get", "f"])
        .assert()
        .success();
    assert_eq!(
        String::from_utf8_lossy(&completing.get_output().stderr),
        "",
        "the credential plugin wrote to the terminal during completion"
    );

    Command::cargo_bin("nfctl")
        .unwrap()
        .env("KUBECONFIG", &kubeconfig)
        .env_remove("NFCTL_FIXTURE")
        .env_remove("NFCTL_CONTEXT")
        .args(["--context", "noisy", "ls"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("PLUGIN-NOISE"));

    std::fs::remove_dir_all(&dir).ok();
}

/// The same broken context on a normal run still has to say what is wrong,
/// or the user never learns to re-authenticate.
#[test]
fn a_normal_run_still_reports_a_broken_context() {
    let kubeconfig = std::env::temp_dir().join("nfctl-completion-silence-kubeconfig");
    Command::cargo_bin("nfctl")
        .unwrap()
        .env("KUBECONFIG", &kubeconfig)
        .env_remove("NFCTL_FIXTURE")
        .env_remove("NFCTL_CONTEXT")
        .args(["--context", "does-not-exist-xyz", "ls"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("nfctl: cluster:"));
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

/// `--timings` writes to stderr, which the TUI is drawing on, so it must not
/// install a subscriber for that one command. The TUI prints the same numbers
/// in its own header instead.
#[test]
fn timings_do_not_write_over_the_tui() {
    use clap::Parser as _;
    use nfctl_cli::Cli;
    let tui = Cli::parse_from(["nfctl", "--timings", "tui"]);
    assert!(tui.owns_the_terminal());
    for cmd in [
        vec!["nfctl", "--timings", "ls"],
        vec!["nfctl", "--timings", "top", "p"],
        vec!["nfctl", "--timings", "status", "p"],
    ] {
        let other = Cli::parse_from(&cmd);
        assert!(!other.owns_the_terminal(), "{cmd:?}");
    }
}
