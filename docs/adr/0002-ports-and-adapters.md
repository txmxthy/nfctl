# 0002 — Ports and adapters, exactly two ports

Status: accepted · 2026-09-05

## Context
Two external systems vary: the Kubernetes API (CRDs, pods, logs) and Numaflow's
per-pipeline daemon (runtime metrics). Everything else (config, clock, terminal)
is stable.

## Decision
`nfctl-core` defines `ClusterPort` and `DaemonPort` as `async_trait` objects, plus a
`DaemonConnector` abstract factory because a daemon is bound to one pipeline.
Adapters (`nfctl-k8s`, `nfctl-daemon`) depend on core; core depends on nothing
with I/O. Adapter DTOs convert into domain types via `TryFrom` (an anti-corruption
layer); kube and HTTP types never cross into core. Services in core compose the
ports and are tested with in-memory fakes.

## Consequences
Every domain identifier is a newtype and every closed set an enum, validated at the
adapter boundary. Adding a third port needs a new ADR; the temptation to abstract
config or the terminal is declined on purpose.
