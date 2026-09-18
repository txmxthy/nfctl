# How edges are drawn

The card view paints edges onto a grid of cells, one cell per terminal
column. orthodag decides where every card sits and where every line turns;
this page describes what nfctl does with that: the painter in
`crates/nfctl-tui/src/cards.rs`, and the few things it decides for itself.

## Tiles

Each cell holds four direction bits: `L`, `R`, `U`, `D`. A bit says "a line
leaves this cell in that direction". The painter has three primitives only:

- a horizontal run sets `R` on every cell but the last and `L` on every cell
  but the first;
- a vertical run does the same with `D` and `U`;
- an arrowhead marks the last cell of a route and is drawn as `▶`, or `▲`
  where a back edge comes up out of its lane, regardless of bits.

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

Colour itself is orthodag's: the tag set of an edge picks a palette slot in
first-encounter order (`orthodag::colour::of`), and the painter maps a slot
to a hue. In the fanout fixture, `even-tag` is slot 0, `odd-tag` slot 1,
`even-tag,odd-tag` slot 2, three distinct colours for three edges out of
`even-or-odd`.

## What orthodag decides

`CardView::new` builds an `orthodag::Graph` from the view graph, one node per
card with a text line for the numbers row and one per row its tags wrap onto,
and asks for boxes of the card's width and at least the card's height. What
comes back is a `Drawing`: a rectangle and column per card, and an orthogonal
polyline per edge with the cell its arrowhead sits in. Columns, the row each
card takes, which row each flow leaves and arrives on, where a long edge runs,
how the verticals in a gap are packed and ordered, and which back edges go
through a lane below the cards are all settled there, and are documented in
that crate (`docs/design.md`, `docs/painter.md`). Two things follow from that:

- Edges carrying the same tags into one card are one line. They take one row
  across every column and one arrowhead, so a card fed by four shards on one
  tag is drawn as one line with four branches into it.
- Two flows never share a vertical. A fan-out leaves as one coloured line per
  tag set, side by side, and a fan-in arrives as one arrowhead per tag set.

## What nfctl decides

The card's width (from its label, its numbers row and the widest tag row,
clamped, and shrunk to a floor when the columns would not fit the panel), the
badges (one per tag combination arriving at a card, in the colour of that
flow), and the painting: the cells above, the crossing style, and the colour
rules. Shard groups are collapsed before any of it, so orthodag sees one node
where the pipeline has `worker-0..worker-2`.

## What this cannot draw

A cell has one colour. Where a horizontal stub out of a card is shared by
two flows it paints in the colour of the branch that reaches furthest, and a
`Mixed` cell is dim. A card with more tag sets than rows grows to hold them,
so the panel scrolls sooner on a busy pipeline than on a quiet one.
