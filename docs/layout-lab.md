# Layout lab

How the card layout is judged and improved without eyes on it. Read this whole
page before changing anything under `crates/nfctl-graph/src/layout/`.

## The vocabulary

A drawn pipeline may use only these shapes:

- An edge is **straight**, an **L** (one 90° bend), or a **Z / S** (two 90°
  bends). Between neighbouring columns nothing else is allowed.
- An edge that **skips** columns may bend at each end: a Z out, a straight run
  across every column it passes, a Z in. Four bends at most, whatever the skip.
- A **back edge** (a cycle) runs through a lane under the cards: four bends.
- An edge touches at most **one junction at each end**: its fork on the source
  bus and its join on the target bus. A bus is one column with one junction on
  the card's middle row; every branch leaves it through its own `├`.
- Two unrelated edges may share a cell only as a **crossing**: one straight
  through horizontally, the other straight through vertically. Anything else
  shared between unrelated edges is an **overlap** and is a defect.
- Fan-outs and fan-ins are **symmetric** around the bus row; spacing is even;
  colour follows the tag combination.

`docs/rendering.md` explains how cells become glyphs.

## The scorer

`nfctl_graph::layout::score(&ViewGraph, &Layout) -> Score` rasterises every
route the way the painter does and counts, per edge: `bends`, `span` (columns
crossed), `allowed` (2 for neighbours, 4 for skips and back edges), `forks`
and `joins` (distinct shared runs with same-source / same-target edges),
`crossings`, `overlaps`, `detour` (vertical cells beyond the row difference).
Per layout it sums them into two tiers:

| tier | fields | rule |
|---|---|---|
| vocabulary | `bends_over_fwd`, `bends_over_skip`, `bends_over_back`, `junction_over`, `overlaps` | must reach 0 and stay there |
| soft | `cross_cells`, `asymmetry`, `detour` | pushed down, never up |

`mixed_cells`, `ink`, `crossings`, `gap_spread`, `width`, `height` are
informational. `total` is `10·bends_over + 10·junction_over + 5·overlaps +
3·cross_cells + 2·asymmetry + detour`.

Two of those need care. `crossings` and the other per-edge counts charge every
edge for its whole path, so two edges drawn as one line pay twice: they see a
merge as a regression. `ink`, the number of cells with an edge glyph in them,
and `cross_cells`, the number of cells where edges actually cross, count what
a reader sees, and are the only metrics that do. Judge a merge by those.

Cell crossings are counted on the drawn geometry. Column order is judged the
same way: `layout` draws every distinct card order the barycenter sweeps
propose (`order.rs::orderings`), refines the best by adjacent card swaps, and
keeps the lowest score; there is no separate inversion count.

## Tools

| command | what | time |
|---|---|---|
| `just layout-score` | every fixture and corpus pipeline, expanded, worst first | 5 s |
| `just layout-score --json PATH` | the same, plus the numbers as JSON | 5 s |
| `cargo test -p nfctl-graph` | layout unit tests, corpus invariants, the quality gate | 30 s |
| `cargo test -p nfctl-tui` | painter goldens and corpus render | 40 s |
| `just lint test` | clippy pedantic and every test | 3 min |
| `cargo run -q -p nfctl-tui --example gallery -- target/gallery/index.html` | regenerate the gallery; an open tab reloads itself | 20 s |

The quality gate (`crates/nfctl-graph/tests/layout_quality.rs`) compares
against `target/layout-lab/baseline.json`. Fixtures must be at zero on the
vocabulary tier outright. The baseline is written only by the person running
the loop, after an accepted change; nothing else touches it.

## Working a round

1. Pick the metric with the largest weighted contribution in `just layout-score`
   and the candidate from the list below that targets it.
2. Change one thing, in one of the files you may touch.
3. `cargo test -p nfctl-graph`, then `cargo test -p nfctl-tui`. A failing
   golden leaves `x.snap.new` next to `x.snap` (there is no `cargo-insta`;
   never set `INSTA_UPDATE` or `INSTA_FORCE_PASS`). Read both. Accept with
   `mv x.snap.new x.snap` only when the new frame is better by the vocabulary
   above, and quote the frame in the commit body. Delete anything left with
   `find crates -name '*.snap.new' -delete` before committing.
4. `just layout-score` and the gate. The vocabulary tier may never rise for
   any pipeline; the soft tier may not rise in total.
5. `just lint test`; the leak grep from the private rules must print exactly
   one line, the repository URL in `Cargo.toml`.
6. Commit `feat(layout): <what>` with the metric deltas in the body, or throw
   the change away: `git checkout -- . && git clean -fd crates/ docs/ && find crates -name '*.snap.new' -delete`.

Every command runs with `GIT_EDITOR=true GIT_PAGER=cat PAGER=cat CARGO_TERM_COLOR=never`.

Files you may change: `crates/nfctl-graph/src/layout/{order,rank,route,tracks,view,tests}.rs`,
`crates/nfctl-tui/src/cards.rs`, `**/tests/snapshots/*.snap`, `docs/rendering.md`,
this page. Files you may not: `score.rs`, `layout_quality.rs`, `layout_score.rs`,
`tests/common/`, any `Cargo.toml` or `Cargo.lock`, `justfile`, `.gitignore`,
`.github/`, `scripts/`, `target/layout-lab/`. Never change scorer
weights, tests or baselines to make a change pass.

Never: switch or check out a branch, push, rebase, amend, stash, `git commit`
without `-m`, `just gallery` (opens a browser), `just gallery-stop`, `pkill`,
background processes (`&`, `nohup`), `cargo insta`, `cargo deny`, `just bench`.
Names: only fixture names and anonymised corpus names (`p1`,
`composite-ns1/p3`) may appear in code, comments, commit bodies or reports; no
absolute paths.

Determinism: the layout must be identical run to run. Any "first", "nearest"
or "median" taken over a `HashMap` must sort first. Layout of the largest
corpus pipeline must stay under 50 ms in debug.

## Candidates, in order

1. **One pass row per edge** (`route.rs::pass_rows`). Choose the row once for
   all of an edge's pass columns: the target's row when the source is a fork
   and the target has one in-edge; the source's row when the target is a join
   and the source has one out-edge; else the source's row. The row must be
   free in every pass column, else the nearest row free in all of them.
2. **Order on real geometry** (`order.rs::reorder`, `mod.rs::layout`). Accept
   the best sweep by cell crossings on the actual geometry plus a penalty per
   edge whose source and target rows differ, not by layer-order inversions.
   Deterministic tie-breaks. Stay inside 50 ms.
3. **Neighbour-median placement** (`route.rs::geometry`). Place cards in
   column order at `max(prev_bottom + gap, desired − h/2)` with `desired` the
   median attach row of already-placed neighbours, instead of centring the
   column on the tallest. Symmetric fan-in. Expect golden churn.
4. **No overlaps** (`tracks.rs::order_tracks`, then `route.rs::pass_rows`).
   A horizontal landing on another run's corner row is a hard constraint where
   a consistent track order exists; when both orders conflict, move the
   landing row.
5. **Pass slots between stacked cards** (`route.rs::geometry`, `pass_rows`).
   Let a long edge run between two stacked cards through a three-row slot
   instead of around the whole stack.
6. **Adjacent-column back edge as a U-turn** (`route.rs`), only when several
   corpus pipelines have cycles.
7. **Bridge crossing style** (`cards.rs`, painter only, no score change): a
   `CrossingStyle::Bridge` that breaks the horizontal one cell either side of a
   true crossing so the vertical reads as passing over; gallery tab
   `cards 220 expanded · bridge`; default stays `Cross`.
8. **Even gap widths** (`route.rs::columns`), informational.

Left for a human: bus colour on fan-in, whether a straight sibling through a
fork bus should draw as `┼`, and the card style (TIM-21).
