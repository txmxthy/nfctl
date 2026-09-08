# How edges are drawn

The card view paints edges onto a grid of cells, one cell per terminal
column. This page describes the painter in `crates/nfctl-tui/src/cards.rs`
and the track packing in `crates/nfctl-graph/src/layout/tracks.rs` that
decides where the verticals go.

## Tiles

Each cell holds four direction bits: `L`, `R`, `U`, `D`. A bit says "a line
leaves this cell in that direction". The painter has three primitives only:

- a horizontal run sets `R` on every cell but the last and `L` on every cell
  but the first;
- a vertical run does the same with `D` and `U`;
- an arrowhead marks the last cell of a route and is drawn as `▶` regardless
  of bits.

A route is an orthogonal polyline, painted as a sequence of runs. Bits are
OR-ed into the cell, so two routes through the same cell leave the union of
their bits. The glyph is a pure function of the bits:

| bits | glyph |
|---|---|
| none | space |
| `L`, `R`, `L R` | `─` |
| `U`, `D`, `U D` | `│` |
| `R D` | `┌` |
| `L D` | `┐` |
| `R U` | `└` |
| `L U` | `┘` |
| `L R D` | `┬` |
| `L R U` | `┴` |
| `U D R` | `├` |
| `U D L` | `┤` |
| anything else | `┼` |

Corners and junctions are never drawn on purpose. A corner is what a
horizontal run and a vertical run leave behind where they meet; `┼` is what
three or four runs leave behind. This is why routes can be painted in any
order and why a fork and a crossing use the same code path.

## Colour

Each cell has one ink. The first route to touch a cell sets it to that
route's colour; a second route with the same colour leaves it alone; a
second route with a different colour sets it to `Mixed`, which paints dim.
An untagged edge has no colour and paints dim too, so `Mixed` and
"untagged" look the same.

One exception. The cell also remembers the colour of the *longest* vertical
run through it (`turn`). When a cell ends up `Mixed`, it paints in that
colour instead. The effect on a fan-out bus: the short horizontal stub from
the card is shared by every branch and stays grey, but the bus itself takes
the colour of the branch that reaches furthest, so it reads as one line from
the source to its far end with the nearer branches peeling off it.

Arrowheads carry their own ink, set by the same rule (`Mixed` if two edges
of different colour end on one head, which is what a fan-in does).

Colour itself is decided in the layout, not the painter: the sorted tag
combination of an edge picks a palette slot in first-encounter order, and
the painter maps a slot to a hue. In the fanout fixture, `even-tag` is
slot 0, `odd-tag` slot 1, `even-tag,odd-tag` slot 2 — three distinct colours
for three edges out of `even-or-odd`.

## Tracks

Within a gap between two columns, every edge that changes row needs a
vertical run. `tracks::pack` assigns those runs to tracks, one column of
cells each, in `(lo, hi, edge)` order. A run may join an existing track when,
against every run already there, one of these holds:

1. they share a source or share a target, whatever their colours;
2. their rows are disjoint (with one row of clearance).

Rule 1 is the bus: every branch out of one card leaves the same column
through its own junction (`├`, or `┼` when a straight edge passes), and every
edge into one card joins the same column before its single head. Rule 2 is
plain packing. A straight edge (source and target on the same row) takes no
track at all.

Gap width is `tracks + 2`, minimum 5, and the tracks sit centred in the gap.
Tracks are then ordered left to right by an adjacent-swap descent over the
number of horizontals that would cross each vertical, with a heavy penalty
when one run's exit row lands on another's corner row (it would read as one
line).

### Two examples from the fixture

`even-or-odd` fans out to `even-sink` (even-tag, cyan), `odd-sink` (odd-tag,
magenta) and `all-sink` (both tags, yellow). Three targets stack in the next
column; the source is centred on the middle one, so the middle edge is
straight and needs no track. The other two touch only on the source row and
share one track by rule 3:

```
   ┌─▶ even-sink      cyan vertical and head
   │
 ──┼─▶ odd-sink       trunk grey, ┼ cyan (turn rule), then magenta
   │
   └─▶ all-sink       yellow vertical and head
```

The two cells of trunk before the `┼` are `Mixed`; the `┼` and the vertical
take the colour of the longest run through them.

`router` in the sharded pipeline fans out to the collapsed `worker` group
(untagged) and to `audit` (tagged). Two runs, no straight edge, one shared
track by rule 1, and the junction is `L U D`:

```
   ┌─▶ worker x3      dim
 ──┤
   └─▶ audit          coloured
```

## What this cannot draw

A cell has one colour. A bus carrying several tag combinations shows the
colour of its furthest branch, not all of them; only from each junction
outward is a branch in its own colour. The alternatives (a wider gap with one
vertical per colour, or one exit row per edge) are written up in the trunk
options note on TIM-21; the single bus was chosen for symmetry.
