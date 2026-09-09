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

## Crossings

Two unrelated edges may share a cell only as a crossing: one straight
through horizontally, the other straight through vertically. The painter
offers two ways to draw it, `CrossingStyle` on the `Palette`, and which one
ships is left for a human:

- `Cross`, the default: the cell is the union of both edges' bits, `───┼───`,
  the same glyph a fork bus leaves where a straight sibling passes through.
- `Bridge`: the vertical reads as passing over. The cell paints `│` in the
  vertical's colour and the horizontal is cut one cell either side, in its
  own colour: `──╴│╶──`. The cuts are made only where the neighbour is a
  plain horizontal run; a corner or junction next to the crossing stays, so a
  horizontal that turns right after the crossing reads `╴│┘`.

Only true crossings change: the painter remembers which edge left which bits
in a cell, and a cell where the two straight runs share a source or a target
is a junction and keeps its `┼`. The gallery shows the bridge on the
`cards 220 expanded · bridge` tab.

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
Tracks are then ordered left to right so that no run's exit row lands on
another run's corner row (it would read as one line) wherever some order
avoids it, and then so horizontals cross as few verticals as possible: every
order is tried for a gap of up to six tracks, an adjacent-swap descent beyond.

No order helps when two cards sit on the rows of two cards in the next column
and are joined crosswise: whichever vertical is left, its exit lands on the
other's corner. `route::build` then shifts a whole column off its neighbours'
rows, by up to half a card step, while that removes an overlap.

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

## Long edges

An edge that skips columns crosses each of them on one pass row, chosen
after the cards are placed. The row it wants is its source's row, or its
target's row when it leaves a fork for a target with a single in-edge (so
the branch bends once, at the bus). The row must be free in every column
the edge passes: no card covers it or the row beside it, and no pass of an
unrelated edge in that column has it. Otherwise the edge takes a free row
between its two ends, nearest the wanted one, and only when there is none a
row outside them. Longer edges pick first. A long edge therefore runs
straight across every column it passes and bends only at its ends.

Two edges out of one card (or into one) may share a pass row when no row of
its own is free without a detour; the shared run reads as their fork or join
continuing across the column.

Stacked cards sit one blank row apart. Where pass slots sit between two
stacked cards, the layout is also built with that gap widened to a margin, a
row per pass and a margin (one row more when the count is even, so the column
stays an odd height), the middle rows held for the edges whose slots sit
there. Two orders are tried: the slots where the barycenter sweep left them,
and the slots moved to the gap they need, which is the gap adding the least
detour nearest the edge's wanted row, taken only when its row is clear of
cards in every other column the edge passes (one row serves them all) and
while the column stays within two rows of the layout's height; a slot with
no such gap goes above or below the stack, whichever is nearer. A widened
layout is kept when it scores lower without raising either tier or the
height past that allowance. That is how a long edge runs between two stacked
cards instead of around the whole stack.

## Fans

A fan-out is symmetric when the source's middle row is the midpoint of the
outermost rows its branches leave the bus on; a fan-in likewise for the rows
its edges join on. The placement sweeps centre a card on the median of its
neighbours, which is the midpoint only when the neighbours are evenly spread,
so the winning layout is refined once more: the sweeps run again with the
midpoint of the extreme neighbour rows as the centre (both roundings, the
grid being odd), then every card whose fan is off centre is nudged towards
the midpoint by up to two rows, the cards stacked beyond it moving along.
When a nudge would push its stack past the layout's height, both run once
more with the layout allowed to grow by two rows, the pass slots' allowance.
Each candidate's geometry is measured first, without drawing, for the
asymmetry and detour it would have; only one that promises less is drawn,
and it is kept only when the drawn total falls without raising either tier
or the height past that allowance. The draws are budgeted by the edges
drawn, so the largest pipeline spends few. Where two stacked cards want the
same row the fans stay as they are: two cards fanning to the same pair, or
fanned into from the same pair, each sit a card's step from the row they
both want, whatever the gap between them.

## What this cannot draw

A cell has one colour. A bus carrying several tag combinations shows the
colour of its furthest branch, not all of them; only from each junction
outward is a branch in its own colour. The alternatives (a wider gap with one
vertical per colour, or one exit row per edge) are written up in the trunk
options note on TIM-21; the single bus was chosen for symmetry.
