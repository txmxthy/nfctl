# Contributing

- `just ci` must pass before opening a PR.
- One PR per roadmap milestone where possible; smaller is fine.
- Architecture decisions go in `docs/adr/`. Read the existing ones first.
- Domain code lives in `nfctl-core` and must not depend on Kubernetes or HTTP crates.
- Test fixtures must be synthetic. Do not commit manifests from real clusters.
- Commit messages: conventional commits, imperative, short body if the *why* is not obvious.
