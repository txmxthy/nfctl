# Contributing

- `just ci` must pass before opening a PR.
- One PR per roadmap milestone where possible; smaller is fine.
- Architecture decisions go in `docs/adr/`. Read the existing ones first.
- Domain code lives in `nfctl-core` and must not depend on Kubernetes or HTTP crates.
- Test fixtures must be synthetic. Do not commit manifests from real clusters.
- Commit messages: conventional commits, imperative, short body if the *why* is not obvious.
- Realistic pipeline shapes come from a private corpus under `testdata/private/` (git-ignored).
  Build your own with `just corpus OUT IN_DIR...` from any Mermaid `graph LR` diagrams; the
  script renames everything and refuses to write output that still contains an input name.
  Corpus tests and benches skip when the directory is absent, so CI never needs it.
