# Demo

Everything here runs against a local cluster only. `just demo-up` installs Numaflow
into the current local context and applies the example pipelines from `examples/`.

Each numbered step is one `vhs` tape in this directory; `just record` renders them.

1. `nfctl ls` — pipelines with phase and health; `-o json | jq` to show it scripts.
2. `nfctl dag fanout-demo` — ASCII DAG; `-o mermaid` for docs.
3. `nfctl logs fanout-demo enrich -f` while the vertex scales to zero and back.
4. `nfctl top fanout-demo -w` — rates, pending, buffer usage updating live.
5. `nfctl pause fanout-demo --wait` then `nfctl resume fanout-demo --strategy slow`.
6. `nfctl apply -f examples/pipelines/fanout-demo-v2.yaml --check` — warns on an edge
   change, blocks on an ISB rename.
7. `nfctl recycle fanout-demo enrich` — rolling restart visible in `top`.

Tapes and example manifests land with milestone M6.
