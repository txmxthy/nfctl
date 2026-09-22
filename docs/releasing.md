# Releasing nfctl

Releases are built by cargo-dist 0.32.0. A version tag such as `v0.1.0` must
match the `nfctl-cli` workspace version. The release workflow first calls the
normal CI workflow, then builds `nfctl` archives for:

- macOS on Apple silicon (`aarch64-apple-darwin`)
- macOS on Intel (`x86_64-apple-darwin`)
- Linux on x86-64 (`x86_64-unknown-linux-gnu`)
- Linux on ARM64 (`aarch64-unknown-linux-gnu`)

Each archive has a SHA-256 checksum. The workflow creates a draft GitHub
release and attaches the artifacts; it does not publish crates, Homebrew
formulae, or the GitHub release. Artifact signing is not configured.

Before tagging, confirm that the standalone-checkout CI gate passes and that
the workspace version and `Cargo.lock` are committed. In particular, release
builds cannot resolve dependencies through paths outside this repository.

To stage a release:

1. Run `dist plan --tag vX.Y.Z` with cargo-dist 0.32.0.
2. Push the matching `vX.Y.Z` tag after normal CI is green.
3. Inspect the draft release, archives, and checksums in GitHub.
4. Publish the draft manually when it is ready for users.

Pull requests run the cargo-dist planning step, but do not build or publish
release artifacts.
