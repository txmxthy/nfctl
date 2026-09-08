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

Each frame has a drawing layer: pick a colour, draw over the frame, add a note,
and `export PNG` (or `export all annotated`) to hand the marked-up frames to
whoever is fixing the layout. Drawings and notes persist in the browser's local
storage per pipeline and tab, so a regenerated page keeps them.
