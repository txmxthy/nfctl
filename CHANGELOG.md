# Changelog

All notable changes to this project are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.1.0](https://github.com/txmxthy/nfctl/releases/tag/v0.1.0) - 2026-09-23

### Added

- *(tui)* a MonoVertex panel draws its chain as one card
- MonoVertex becomes a Workload kind
- *(tui)* one column of edges, a card that fits its tags, and a ticker for what does not
- *(cli)* redirect stderr and the TUI writes its timings there
- *(cli)* --timings says where a command spent its time
- *(gallery)* draw and annotate frames, export marked-up PNGs
- isb namespace column, rolling vertex recycle
- *(cli)* complete resource names from the cluster
- *(graph,tui,cli)* layered card layout, shard collapse, tag colours
- lists span all namespaces; bare names resolve across them
- --fixture, golden TUI frames, cluster-free recordings
- nfctl map prints the whole command surface as a tree
- MonoVertex support (mvtx ls/get/status/logs/pause/resume)
- nfctl tui
- pause, resume, recycle, wait, scale and apply --check
- status, top and isb commands
- nfctl logs with churn-safe multi-pod tailing
- nfctl dag with mermaid, dot and box-drawing output
- domain model, ports, fakes, ls/get

### Fixed

- follow-ups from the hardening review ([#3](https://github.com/txmxthy/nfctl/pull/3))
- *(cli)* --timings does not scribble over the TUI
- *(cli)* a Tab press never prints a credential plugin's errors

### Other

- prepare nfctl for its alpha release ([#6](https://github.com/txmxthy/nfctl/pull/6))
- *(release)* automate binary releases ([#2](https://github.com/txmxthy/nfctl/pull/2))
- Harden nfctl for an open-source release
- *(core)* ask the daemon its four questions at once, and write timings to a file
- *(daemon)* keep the daemon client, and read the pipeline once
- demo pipelines, vhs tapes and recordings, README
- scaffold workspace, lints, ci, adrs
- nfctl

### Added
- Workspace scaffold, lints, CI, ADRs 0001–0008.
- Domain model with validated identifiers and topology; `ClusterPort`/`DaemonPort` ports; in-memory fakes.
- `nfctl ls`, `nfctl get`, `nfctl completions`; table, wide, JSON and YAML output; every other command is a visible stub (exit 4).
- `nfctl dag`: box-drawing, Mermaid or DOT rendering of a pipeline topology; ASCII fits the terminal width.
- `nfctl logs`: multi-pod log tailing tagged by pod and container; `-f` follows through pod replacement and container restarts, resuming from the last timestamp seen.
- `nfctl status` / `nfctl top`: pipeline phase and health fused with per-vertex rates and pending and per-edge buffer usage and watermark lag, read from the pipeline daemon through an automatic port-forward. `--daemon-url` for in-cluster use.
- `nfctl isb ls` / `isb inspect`.
- Lifecycle: `pause --wait` (reports drain), `resume --strategy fast|slow`, `recycle` (vertex pods or pause-drain-resume), `wait --phase`, `scale`; every mutating verb takes `--dry-run`.
- `nfctl apply`: server-side apply with `--check`, which refuses immutable changes (ISB, instance, vertex type, reduce partitions) and warns on topology or image changes that risk in-flight data.
- `nfctl tui`: pipelines list, pipeline detail with vertex cards laid out by rank and live rates/pending, and a following log view. One worker task owns all I/O; panels are message-driven.
- `nfctl mvtx status|logs|pause|resume`: MonoVertex support, including its daemon (metrics and health) and its pause/resume semantics.
- A MonoVertex is a kind, not a separate hierarchy (ADR 0009): `ls` lists both kinds with a `KIND` column, `get <name>` resolves either and names both candidates when a name is ambiguous, and the TUI list shows both and opens a MonoVertex panel with no flow drawing or edge table. `ls -o json|yaml` now emits objects carrying a `kind` field; `mvtx ls` and `mvtx get` are gone, replaced by the unified pair.
- `nfctl map`: the whole command surface as one tree (or JSON), for pruning.
- Lists span all namespaces unless `-n` is given, and single-resource commands resolve a bare name across namespaces (error if ambiguous). Fits a namespace-per-pipeline layout.
- TUI: every edge is drawn (fan-out, fan-in, column-skipping and back edges); tables size columns to their content; log lines wrap.
- Mermaid importer (`from_mermaid`), an anonymiser script for real diagrams, corpus tests that run only when a private `testdata/private/` exists, criterion benches, and `just corpus-fixture`/`corpus-render` helpers.
- Card view: a layered layout (columns by rank, long edges straight through pass rows, barycenter ordering, edges into or out of one vertex bundled onto one track, back edges in lanes) with shard groups collapsed to one `name ×N` card (`x` expands), tags merged into a badge row on the destination card, edges coloured by tag combination, and horizontal scrolling instead of squeezed cards. `dag` collapses shards too (`--expand-shards`) and colours its box-drawing output on a terminal; `--no-color` and `NO_COLOR` turn colour off everywhere.
- Dynamic completions: `nfctl get <Tab>` offers pipeline names (with phase and namespace), `logs <pipeline> <Tab>` its vertices, `mvtx`/`isb` their resources, `-n` the namespaces seen. Names come from the fixture on the line, a 30-second cache under `$XDG_CACHE_HOME/nfctl`, or one bounded cluster call. `nfctl completions <shell>` now prints the dynamic registration snippet.
- `isb ls` shows the namespace and `isb inspect` refuses an ambiguous name across namespaces. `recycle <pipeline> <vertex>` restarts pods one at a time, waiting for each replacement (`--all-at-once` for the old behaviour).
- The `MonoVertex` panel draws the container chain (ADR 0010): one card for the unit, with the container chain inside it — a box per stage joined by arrows, and a dashed hop to the fallback sink — the replica badge in the top border and the rate and pending count in the bottom one. Width steps down a three-rung ladder (boxes and full labels, boxes and short labels, bare labels) so the card floats in the middle of the panel rather than overflowing or sitting in a corner. `MonoVertex` gained `has_fallback`, read from `spec.sink.fallback`.
- `--fixture FILE`: run any command, including the TUI, against in-memory fakes loaded from a YAML/JSON fixture. Golden-frame tests and cluster-free recordings use the same file.
- Demo: `just demo-up/down`, synthetic example pipelines under `examples/`, vhs tapes and recordings under `docs/demo/`.

### Changed
- `status`/`top` JSON and YAML: `buffers[].from` (one vertex) is now `buffers[].sources` (every vertex feeding that buffer). A fan-in buffer is listed on each edge that feeds it.
- `--daemon-url https://...` verifies the server name and certificate against the platform trust store. `--daemon-insecure` accepts any certificate, for a port-forward you opened yourself.
- `resume`, `wait`, `scale`, `apply`, `logs` and `mvtx pause|resume` honour `-o json|yaml`; `completions` and `tui` refuse them (exit 2).
- `top --interval` no longer clears the screen when stdout is not a terminal.
- `logs -c NAME` fails with the containers that do exist when `NAME` is on none of the matching pods; under `--follow` with no pods yet it waits.
- Manifests with an unknown `desiredPhase`, `onFull`, tag operator, resume strategy or zero partitions are rejected (exit 3) instead of being read as a default.
- Daemon responses over 8 MiB are refused.
