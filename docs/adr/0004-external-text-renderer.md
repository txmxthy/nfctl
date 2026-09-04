# 0004 — An external library draws the text

Status: accepted · 2026-09-05

## Context
`nfctl dag` needs a terminal rendering of a pipeline: boxes for vertices,
orthogonal lines for edges, tag labels on the edges, fitted to the terminal's
width. Writing a layered layout and a box-drawing painter inside a Kubernetes
CLI would double the size of the crate that owns pictures, and none of it is
about Numaflow.

## Decision
Emit Mermaid from the domain `Topology` with our own code, and draw the text
with `orthodag`, a box-drawing DAG renderer maintained alongside this tool. It
takes a graph with tagged edges, returns text or styled spans, and has no
terminal dependency of its own. Mermaid text stays a first-class output for
docs.

## Consequences
The graph crate is an adaptor: it turns a `ViewGraph` into an `orthodag::Graph`
and maps palette slots to ANSI hues. Layout quality is orthodag's concern and is
measured there. The same library can hand back its geometry, so the TUI's card
view can share the layout later rather than growing one of its own.
