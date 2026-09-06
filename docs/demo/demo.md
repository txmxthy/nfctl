# Demo

Everything here runs against a local cluster only. `just demo-up` installs
Numaflow into the current local context and applies `examples/`.

```
just demo-up      # Numaflow + ISB + four example pipelines, waits for Running
just record       # renders docs/demo/*.tape to gifs with vhs (needs `nfctl` on PATH)
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

The gifs next to the tapes are what `just record` produced last. The tapes assume a shell with `nfctl` on PATH and the local context selected
(`NFCTL_CONTEXT=<local-context>` works too).
