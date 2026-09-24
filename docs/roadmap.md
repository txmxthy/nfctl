# Roadmap

| Command | Status | Milestone |
|---|---|---|
| `ls`, `get` (pipelines and MonoVertices, with a KIND column) | done | M1 |
| `completions` | done | M1 |
| `map` | done | — |
| `dag` | done | M2 |
| `logs` | done | M3 |
| `top`, `status` | done | M4 |
| `isb ls`, `isb inspect` | done | M4 |
| `pause`, `resume`, `wait` | done | M5 |
| `recycle` | done | M5 |
| `apply --check` | done | M5 |
| `scale` | done | M5 |
| `tui` | done | M7 |
| `tui`: MonoVertex card with its container chain (ADR 0010) | done | — |
| `mvtx status/logs/pause/resume` (`ls`/`get` unified away, ADR 0009) | done | M8 |
| unify `mvtx status/logs/pause/resume` into the shared verbs | planned | later |
| corpus tests + benches (private seed) | done | M9 |
| `tui`: shard collapse (`x`), edge bundling, tag colours, badges, scroll | done | M10 |
| `dag --expand-shards`, coloured `dag -f ascii` | done | M10 |
| `completions` from live resources (dynamic) | done | M11 |
| docs + ADRs 0006–0008 | done | M12 |
| `isb ls` namespace column, rolling `recycle` | done | M13 |
| TUI load timings and concurrent detail-panel metrics | done | M14 |
| layout gallery (`just gallery`) | done | — |
| straight chains, no framed cards, short back edges | planned | M15 |
| ServingPipeline | planned | later |
| prebuilt binaries, attestations and Homebrew tap | done | release |

Statuses: `done` · `wip` · `stub` (command exists, exits 4 with this pointer) · `planned`.
