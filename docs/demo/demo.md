# Demo

Two kinds of tape live here. Tapes that set `NFCTL_FIXTURE` run against
`examples/fixtures/demo.yaml` and need no cluster: they are deterministic, render
in CI, and their `Screenshot` lines produce the PNG stills the README embeds.
The rest run against the local demo cluster.

Everything cluster-bound runs against a local cluster only. `just demo-up` installs
Numaflow into the current local context and applies `examples/`.

```
just demo-up      # Numaflow + ISB + four example pipelines, waits for Running
just record          # every tape (cluster ones need `just demo-up` first)
just record-fixture  # only the fixture-driven tapes: no cluster needed
just demo-down    # removes everything again
```

Script, one tape per step:

1. `ls` — pipelines with phase; `-o json | jq` shows it scripts.
2. `dag` — the fanout pipeline as box-drawing text, then as Mermaid.
3. `logs` — every pod of a vertex, tagged; `-f` keeps following through pod replacement.
4. `top` — rates, pending, buffer usage and watermark lag refreshing live.
5. `pause --wait` → `status` → `resume --strategy slow`: a drained pause and a slow ramp-up.
6. `apply --check` on two edits of the fanout pipeline: one warns, one is refused (exit 3).
7. `recycle` a vertex and watch the replacement pod pick up in `logs -f`.
8. `tui`: list → detail → vertex logs and back.

The gifs next to the tapes are what `just record` produced last. The tapes assume a shell with `nfctl` on PATH and the local context selected
(`NFCTL_CONTEXT=<local-context>` works too).
