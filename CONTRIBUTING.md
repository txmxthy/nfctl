# Contributing

- `just ci` must pass before opening a PR.
- One PR per roadmap milestone where possible; smaller is fine.
- Architecture decisions go in `docs/adr/`. Read the existing ones first.
- Domain code lives in `nfctl-core` and must not depend on Kubernetes or HTTP crates.
- Test fixtures must be synthetic. Do not commit manifests from real clusters.
- Commit messages: conventional commits, imperative, short body if the *why* is not obvious.
- Realistic pipeline shapes come from a private corpus under `testdata/private/` (git-ignored).
  Build your own with `just corpus OUT IN_DIR...` from any Mermaid `graph LR` diagrams; the
  script renames everything and refuses to write output that still contains an input name.
  Corpus tests and benches skip when the directory is absent, so CI never needs it.

## Looking at layouts

`just gallery` renders every fixture pipeline (and the private corpus when
present) through the card view at three widths, collapsed and expanded, and
through `dag`, into one HTML page with the terminal's colours. Use it to review
a layout change across the whole suite before trusting the goldens;
`just gallery target/gallery/public.html --public` makes a shareable page
without the corpus.

Each frame has a drawing layer: pick a colour, draw over the frame, add a note.
`just gallery` also starts a small notes server (`scripts/gallery-notes.py`),
so every stroke and note is written to `target/gallery/notes.json` and
`export PNG` drops the marked-up frame into `target/gallery/png/`, where the
person fixing the layout can read them straight from the checkout. Without the
server the page keeps them in the browser instead. `just gallery-stop` ends it.
Annotations belong to the build they were drawn on: when the page is
regenerated (it reloads itself), older ones move under `~stale` in the JSON
and stop being painted, so a frame never shows marks made on a previous
layout.
