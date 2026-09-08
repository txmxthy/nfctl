# Architecture

```mermaid
flowchart LR
    cli[nfctl-cli] --> core[nfctl-core]
    tui[nfctl-tui] --> core
    cli --> graph[nfctl-graph]
    graph --> core
    k8s[nfctl-k8s] --> core
    daemon[nfctl-daemon] --> core
    k8s -. kube-rs .-> apiserver[(Kubernetes API)]
    daemon -. HTTPS/JSON over port-forward .-> dsvc[(pipeline daemon :4327)]
```

Dependency direction is always toward `nfctl-core`. The composition root that picks
adapters lives in `nfctl-cli`.

## Ports

| Port | Owner | Implemented by | Purpose |
|---|---|---|---|
| `ClusterPort` | core | `nfctl-k8s` | list/watch CRDs, patch lifecycle, pods, logs |
| `DaemonPort` | core | `nfctl-daemon` | buffers, rates, pending, watermarks, health |
| `DaemonConnector` | core | `nfctl-daemon` | abstract factory: one `DaemonPort` per pipeline |

Only these vary between environments, so only these are abstracted. Config, clock and
terminal are used directly.

## Services

`PipelineService` composes the two ports and owns every multi-step procedure
(`status` fusion, `pause` with drain wait, `recycle`, `check_apply`). `LogTailer`
supervises one tail per (pod, container) under a pod watch so output survives
scale-to-zero. Both are tested against in-memory fake adapters.

## Data flow for `nfctl top`

1. `ClusterPort::get_pipeline` → domain `Pipeline` (validated `Topology`).
2. `DaemonConnector::connect` → `DaemonPort` bound to that pipeline.
3. `DaemonPort::health/vertex_metrics/buffers` → joined per vertex and edge.
4. Renderer prints a table or JSON. Daemon failure degrades to the CRD-only view with a warning.

## One fixture, three consumers

`examples/fixtures/demo.yaml` is a serialised slice of the domain: pipelines,
MonoVertices, ISB services, per-pipeline daemon data and pods with log lines.
`nfctl --fixture FILE` runs any command against in-memory fakes built from it.
The TUI's golden-frame tests (`crates/nfctl-tui/tests/frames.rs`, ratatui
`TestBackend` + `insta`) render the panels from the same file, and the vhs tapes
under `docs/demo/` that set `NFCTL_FIXTURE` record the same screens as GIFs and
PNGs. Changing the fixture changes all three together.

## Drawing a pipeline

`nfctl-graph` owns every picture. The Mermaid and DOT emitters are pure functions
of the topology. `layout` is a pure layered layout for the card view: columns by
rank (back edges removed first), one-row pass slots for edges that skip columns,
barycenter ordering, edges sharing a source or a target bundled onto one track
in the gap, back edges through lanes under the cards. It works on a `ViewGraph`,
which is the topology with shard groups (`name-0..name-N` with identical
neighbourhoods) collapsed to one node. Edge colour is decided once, in the
layout, by tag combination; the TUI painter and the CLI's ANSI writer only map
a palette index to a hue.

The box-drawing `dag` output is drawn by orthodag (ADR 0004), whose styled spans
say which flow painted each cell, so the CLI can colour it. The layout itself is ADR 0006; completions are ADR 0007. How cells, colours and tracks turn into glyphs is in [rendering.md](rendering.md).

## Completions

`nfctl` completes its own resource names. The shell snippet from
`nfctl completions <shell>` calls the binary back with `COMPLETE=<shell>` and the
line so far; `main` answers that before anything else runs. Each positional has
a completer in `crates/nfctl-cli/src/complete.rs` that reads the line
(`--context`, `-n`, `--fixture`, the pipeline already typed) and asks a catalog
built from the fixture, a cache under `$XDG_CACHE_HOME/nfctl` younger than 30 s,
or one cluster round-trip bounded to 1.5 s. A completer cannot report errors, so
every failure degrades to the stale cache or an empty list.
