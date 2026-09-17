# 0010 — A MonoVertex's stages are drawn, but never as vertices

Status: accepted · 2026-09-17

Supersedes one sentence of [0009](0009-monovertex-is-a-kind.md): that a
MonoVertex panel is "just the header and the one row of numbers its daemon
answers with".

## Context
0010 was right that a MonoVertex has no topology and wrong that it has no
structure. Its spec carries a container chain — a source, an optional
transformer, an optional map, a sink, and optionally a fallback the sink writes
to when it fails — and `has_transformer` / `has_map` were already parsed from
the spec and rendered nowhere. The panel was a full-screen box holding a
five-column table with one row in it, pinned to the top-left corner, which told
an operator less about the workload than `kubectl get` does.

The obvious cheap fix is to synthesise a `Topology` from the stages and feed it
to the layered card layout the pipeline panel uses, which would cost about a
hundred lines and reuse the whole painter. It would also be a lie. A vertex
card in that drawing means a workload that scales on its own, reads from an ISB
buffer, and reports its own rate, pending count and watermark. A MonoVertex's
stages are containers in one pod: they share a process and a replica count,
nothing buffers between them, and the daemon answers one set of numbers for the
whole unit. Drawing them as cards would invite every question the model cannot
answer — what is this stage's pending count, why can I not scale it — and the
absence of edge labels and buffer figures would read as missing data rather
than as data that does not exist.

## Decision
One card per MonoVertex, because the card is the thing that has a phase, a
replica count and a rate. The container chain is drawn inside it, each stage in
a box of its own joined by arrows, with the fallback's hop dashed since nothing
travels it unless the primary sink fails. The replica badge is worked into the
top border and the throughput into the bottom one, so neither costs an interior
row. The stack floats in the middle of the panel.

The boxes are safe here only because they are nested. A box floating in a flow
pane promises a vertex; a box inside a frame that carries the name, the replica
badge and the single rate reads as a container within one pod, which is what it
is. So no stage gets a buffer figure, a watermark or a number of its own, and
the card layout engine is not involved — `nfctl-graph` stays a pipeline
concern.

Width is a three-rung ladder, widest that fits: boxes with spelled-out labels,
boxes with abbreviated ones, then bare abbreviated labels with no boxes. The
frame never overflows its room.

`MonoVertex` gains `has_fallback`, parsed from `spec.sink.fallback`, which the
DTO previously ignored.

## Consequences
`ls -o json|yaml` emits `has_fallback` on every MonoVertex; anything reading
that output sees one more field. The stage row is structure read from the spec,
so it is correct whether or not the daemon answers, and it says nothing about
where a backlog actually sits — the one pending figure is the unit's, and an
operator wanting to know which container is slow still has to read logs. If
Numaflow later gives MonoVertex per-stage metrics, this card is the wrong shape
for them and the decision is worth reopening.
