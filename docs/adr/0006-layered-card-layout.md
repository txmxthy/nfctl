# 0006 — Layered layout for the card view

Status: superseded · 2026-09-07

## Context
The TUI's first card view placed cards by rank and drew each edge through the
middle of the gap, or through a lane under the cards when it skipped a column.
On real pipelines (fan-out to shards, edges spanning several columns, cycles)
that gave one card per shard, full-width lanes for every long edge, and no way
to tell one edge from another. orthodag already solves the same problems for
`dag`, but it owns its canvas and cannot draw our cards.

## Decision
`nfctl-graph::layout` is a pure Sugiyama-style layout over a `ViewGraph`: DFS
colouring removes back edges, longest path assigns columns, an edge spanning k
columns gets k−1 one-row pass slots so it runs straight between cards,
barycenter sweeps order each column, and within a gap the vertical runs are
packed onto tracks that may be shared only by edges with the same source or the
same target (or disjoint rows), so a fan-out leaves as one trunk and a fan-in
arrives as one head. Back edges take a lane row under the cards. Shard groups
(`name-0..name-N` with identical outside neighbourhoods and tags, per-shard tag
suffixes normalised) collapse to one node by default. Colour is decided in the
layout by sorted tag combination; renderers only map an index to a hue. Tags
merge into a badge row on the destination card rather than inline labels.

Wide pipelines scroll a column at a time; cards shrink to a floor and never
below it, so numbers stay readable.

## Consequences
The layout is testable without a terminal and shared by the TUI painter and
the corpus tests. Crossings are minimised heuristically, not optimally; the
corpus keeps that honest. Inline edge labels are gone: a reader looks at the
badge on the card an edge arrives at, the same rule the reference viewer this
was modelled on uses.

## Superseded
orthodag now places the cards and routes the lines (ADR 0004): `nfctl-tui`
hands it a box per node sized to the card's text and reads the boxes and
polylines back. Colour by tag combination, badges on the destination card,
shard collapsing and the card widget itself are unchanged and still nfctl's.
