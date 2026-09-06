# Changelog

All notable changes to this project are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- Workspace scaffold, lints, CI, ADRs 0001–0005.
- Domain model with validated identifiers and topology; `ClusterPort`/`DaemonPort` ports; in-memory fakes.
- `nfctl ls`, `nfctl get`, `nfctl completions`; table, wide, JSON and YAML output; every other command is a visible stub (exit 4).
- `nfctl dag`: box-drawing, Mermaid or DOT rendering of a pipeline topology; ASCII fits the terminal width.
- `nfctl logs`: multi-pod log tailing tagged by pod and container; `-f` follows through pod replacement and container restarts, resuming from the last timestamp seen.
