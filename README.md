# nfctl

A CLI (and soon TUI) for operating [Numaflow](https://numaflow.numaproj.io/) pipelines.

`kubectl` can create and patch Numaflow resources, but it cannot tail a vertex's logs
through pod churn, show buffer fill and processing rates next to the pipeline phase,
draw the DAG, or run a safe pause → drain → resume. `nfctl` does those things in one
static binary.

> Status: pre-alpha. See [docs/roadmap.md](docs/roadmap.md) for what works today.

## Planned commands

```
nfctl ls                          pipelines in a namespace, with health
nfctl dag <pipeline>              ASCII / mermaid / dot rendering of the topology
nfctl logs <pipeline> [vertex]    multi-pod tail that survives scale-to-zero
nfctl top <pipeline>              phase + rates + pending + buffer usage, live
nfctl pause | resume | recycle    lifecycle procedures done the right way
nfctl apply -f spec.yaml --check  refuse or warn on changes that need delete-and-recreate
```

Every command supports `-o json`; every mutating command supports `--dry-run`.

## Design

Hexagonal: a pure domain crate with two ports (cluster, daemon), thin adapters for
kube-rs and the Numaflow daemon API, and a CLI on top. See
[docs/architecture.md](docs/architecture.md) and the ADRs in [docs/adr](docs/adr).

## Development

```
just ci          # fmt, clippy -D warnings, tests, cargo-deny
just demo-up     # local cluster with Numaflow + example pipelines (see docs/demo)
```

## Licence

Apache-2.0. The box-drawing output is drawn by the orthodag crate (MIT OR
Apache-2.0), a sibling checkout the graph crate depends on by path.
