# Releasing nfctl

Release-plz prepares releases from Conventional Commit squash titles. Changes
to any workspace crate are included in the single `nfctl` version and
changelog. A `fix`, `feat`, `perf` or `refactor` change opens or updates the
release PR; a breaking `!` change receives the corresponding Cargo SemVer bump.
CI and documentation changes alone do not cut a release.

After the release PR is squash-merged, release-plz creates the matching
`vX.Y.Z` tag. Cargo-dist 0.33.0 then runs normal CI and builds archives for:

- macOS on Apple silicon (`aarch64-apple-darwin`)
- macOS on Intel (`x86_64-apple-darwin`)
- Linux on x86-64 (`x86_64-unknown-linux-gnu`)
- Linux on ARM64 (`aarch64-unknown-linux-gnu`)

Each archive has a SHA-256 checksum and a GitHub artifact attestation. The
workflow also generates the Homebrew formula and creates a draft GitHub
release. The workspace crates are not published to crates.io.

To release:

1. Review and squash-merge the release-plz PR after its required checks pass.
2. Inspect the resulting draft release, archives, checksums and attestations.
3. Run the **Publish release** workflow with the release tag.
4. Confirm the **Homebrew** workflow updates `txmxthy/homebrew-tap`.
5. Install with `brew install txmxthy/tap/nfctl` and run `nfctl --version`.

Pull requests run the cargo-dist plan but do not build or publish artifacts.
Prereleases never update the stable Homebrew formula.
