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
- `nfctl status` / `nfctl top`: pipeline phase and health fused with per-vertex rates and pending and per-edge buffer usage and watermark lag, read from the pipeline daemon through an automatic port-forward. `--daemon-url` for in-cluster use.
- `nfctl isb ls` / `isb inspect`.
- Lifecycle: `pause --wait` (reports drain), `resume --strategy fast|slow`, `recycle` (vertex pods or pause-drain-resume), `wait --phase`, `scale`; every mutating verb takes `--dry-run`.
- `nfctl apply`: server-side apply with `--check`, which refuses immutable changes (ISB, instance, vertex type, reduce partitions) and warns on topology or image changes that risk in-flight data.
- `nfctl tui`: pipelines list, pipeline detail with vertex cards laid out by rank and live rates/pending, and a following log view. One worker task owns all I/O; panels are message-driven.
- `nfctl mvtx ls|get|status|logs|pause|resume`: MonoVertex support, including its daemon (metrics and health) and its pause/resume semantics.
- `nfctl map`: the whole command surface as one tree (or JSON), for pruning.
- Lists span all namespaces unless `-n` is given, and single-resource commands resolve a bare name across namespaces (error if ambiguous). Fits a namespace-per-pipeline layout.
- `--fixture FILE`: run any command, including the TUI, against in-memory fakes loaded from a YAML/JSON fixture. Golden-frame tests and cluster-free recordings use the same file.
- Demo: `just demo-up/down`, synthetic example pipelines under `examples/`, vhs tapes and recordings under `docs/demo/`.
