# nfctl

A CLI and TUI for operating [Numaflow](https://numaflow.numaproj.io/) pipelines.

`kubectl` can create and patch Numaflow resources, but it cannot follow a vertex's
logs through pod churn, show buffer fill and processing rates next to the pipeline
phase, draw the DAG, or run a safe pause → drain → resume. `nfctl` does those
things in one static binary, with `-o json` on every command and `--dry-run` on
every mutating one.

> Status: pre-alpha. Commands below work against Numaflow 1.6+. ServingPipeline is
> not covered yet; see [docs/roadmap.md](docs/roadmap.md).

## Commands

```
nfctl ls                                  pipelines in a namespace (-A for all)
nfctl get <pipeline> [-o wide|json|yaml]
nfctl dag <pipeline> [-f ascii|mermaid|dot]
nfctl logs <pipeline> [vertex] [-f] [-c container] [--since 10m] [--tail N]
nfctl status <pipeline>                   phase + health + rates + pending + buffer usage
nfctl top <pipeline> [-i 2]               the same, refreshing
nfctl isb ls | isb inspect <name>
nfctl pause <pipeline> [--wait]           reports whether buffers drained
nfctl resume <pipeline> [--strategy fast|slow]
nfctl recycle <pipeline> [vertex]         restart a vertex's pods, or pause-drain-resume
nfctl wait <pipeline> --phase paused
nfctl scale <pipeline> <vertex> <n>
nfctl apply -f spec.yaml [--check]        refuses changes that need delete-and-recreate
nfctl mvtx ls|get|status|logs|pause|resume  MonoVertex equivalents
nfctl tui                                 the same, interactive
nfctl completions <shell>
nfctl map                                 every command and option as one tree
```

Global flags: `-n/--namespace`, `-A`, `--context`, `--request-timeout`, `-o`,
`--daemon-url` (skip the port-forward when running in-cluster).

### `dag`

```
                                                ┌───────────┐
                                             ┌─▶│ even-sink │
                                             │  └───────────┘
           ┌─────────────┐                   │
┌────┐     │ even-or-odd │─even-tag──────────┘  ┌───────────┐
│ in │────▶│             │─odd-tag─────────────▶│ odd-sink  │
└────┘     │             │─even-tag, odd-tag─┐  └───────────┘
           └─────────────┘                   │
                                             │  ┌───────────┐
                                             └─▶│ all-sink  │
                                                └───────────┘
```

### `status`

```
default/linear  phase=Running  health=healthy  desired=Running
Pipeline data flow is healthy (D1)

VERTEX  KIND    PARTS  RATE/1m  RATE/5m  PENDING
in      source  1      5.0      5.0      -
cat     map     1      5.0      5.0      4
out     sink    1      4.9      5.0      9

EDGE        PENDING  ACK-PENDING  USAGE  FULL  WATERMARK
in -> cat   0        6            0%     no    1s ago
cat -> out  0        10           0%     no    2s ago
```

Runtime numbers come from the pipeline's daemon, reached through an automatic
port-forward; if the daemon is unreachable the CRD half still renders with a warning.

## Install

```
cargo install --path crates/nfctl-cli
```

Prebuilt binaries and a Homebrew tap are on the roadmap.

## Demo

Every command also runs against a fixture file instead of a cluster:

```
nfctl --fixture examples/fixtures/demo.yaml tui
```

The same fixture drives the golden-frame tests, the recordings below and the
stills used to review layout, so what is tested is what is shown.

| | |
|---|---|
| ![pipelines](docs/demo/tui-pipelines.png) | ![detail](docs/demo/tui-detail.png) |
| ![status](docs/demo/status.png) | ![dag](docs/demo/dag.png) |

Animated: [tui](docs/demo/tui.gif) · [ls](docs/demo/ls.gif) · [logs through a pod
restart](docs/demo/logs.gif) · [top](docs/demo/top.gif) · [pause and resume](docs/demo/pause.gif)
· [apply --check](docs/demo/apply.gif) · [recycle](docs/demo/recycle.gif)

`just demo-up` installs Numaflow into your current local kube context and applies
the pipelines in [`examples/`](examples); [`docs/demo`](docs/demo) has the tapes
(`just record-fixture` renders the cluster-free ones).

## Design

Hexagonal. A pure domain crate with two ports (cluster, daemon), thin adapters for
kube-rs and the daemon's JSON API, and a CLI on top. See
[docs/architecture.md](docs/architecture.md) and the ADRs in [docs/adr](docs/adr).

## Development

```
just ci          # fmt, clippy -D warnings, tests, cargo-deny
just test-live   # tests that need the demo cluster
```

## Licence

Apache-2.0. The box-drawing output is drawn by the orthodag crate (MIT OR
Apache-2.0), a sibling checkout the graph crate depends on by path.
