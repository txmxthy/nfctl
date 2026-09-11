# 0009 — A MonoVertex is a kind, not a parallel hierarchy

Status: accepted · 2026-09-12

## Context
A MonoVertex is a different Kubernetes resource from a Pipeline, but to an
operator it is the same thing: something named in a namespace that runs, pauses
and fails. The tool nonetheless had two of everything — `ls` beside `mvtx ls`,
`get` beside `mvtx get`, a list panel that only knew pipelines — so anyone
looking for a workload had to know its kind before they could ask about it.
That is the wrong way round: the kind is what you find out, not what you supply.

## Decision
Keep both domain types (they are genuinely different CRDs, with different
spec, status and daemon capabilities) and unify the surface above them.

`Workload` is an enum over `Pipeline` and `MonoVertex` with the accessors both
share: key, namespace, name, phase, desired phase, vertex count, message, age.
`WorkloadKind` names the two; `WorkloadKey` is a namespace, a name and a kind,
and converts to a `PipelineKey` or a `MonoVertexKey` for the ports.
`WorkloadPhase` is the union of the two controllers' phases, so a reader does
not have to know that only a Pipeline reports `Pausing`.

The ports are unchanged: `list_workloads` and `get_workload` are use cases on
`PipelineService`, composed from the two existing `ClusterPort` calls (asked
concurrently, since they are independent round trips). Adapters gained nothing
to implement.

`nfctl ls` lists both with a `KIND` column; `-o wide` adds each kind's own
columns with `-` where the other has none. `nfctl get <name>` resolves across
both and reports an ambiguous name with the kind and namespace of every
candidate. In the TUI the list shows both, and selecting a MonoVertex opens a
panel of its own: it has no topology, so no flow drawing and no edge table,
just the header and the one row of numbers its daemon answers with.

`mvtx ls` and `mvtx get` are removed — they are exactly what the unified pair
now does. `mvtx status`, `logs`, `pause` and `resume` stay: unifying those
means one screen and one set of flags covering two lifecycles (a pipeline's
pause drains buffers and its resume takes a strategy; a MonoVertex's does
neither), which is a decision of its own.

## Consequences
`ls -o json|yaml` now emits `Workload`, an object per entry with a `kind` field
alongside the resource's own fields, instead of a bare array of pipelines; `get`
does the same for one. Anything parsing that output has to read past `kind`.
`ls` and `get` cost two API calls where they cost one, run in parallel; `get`
now lists even when `-n` is given, because the kind has to be discovered before
the resource can be read. A name shared by a Pipeline and a MonoVertex in one
namespace is ambiguous to `get`, and the error names both — `-n` cannot resolve
that one, and the kind-scoped `mvtx` verbs are the way through it until a
`--kind` flag earns its place.
