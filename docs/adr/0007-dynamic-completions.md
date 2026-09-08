# 0007 — Complete resource names from the cluster

Status: accepted · 2026-09-08

## Context
Every command wants a pipeline, vertex, MonoVertex or ISB name. A static
completion script knows the flags but not the names, so the operator ends up
running `nfctl ls` first.

## Decision
Use clap's dynamic completion (`clap_complete` `unstable-dynamic`): the
registration snippet calls `nfctl` back on every Tab with the line typed so far,
and a completer attached to each positional answers from a catalog. The catalog
comes from the fixture named on the line, else a per-context cache younger than
30 seconds, else one cluster round trip bounded to 1.5 seconds, else the stale
cache, else nothing. A completer cannot report errors, so it never tries.
`main` answers completion requests before starting the async runtime.

The API is behind an unstable feature, so `clap` and `clap_complete` are pinned
to `~4.6`, and `nfctl completions <shell>` prints the snippet rather than a
generated script so an upgrade cannot silently break sourced files.

## Consequences
Tab is instant when warm and bounded when cold; a cluster that is down costs at
most 1.5 seconds once per 30 seconds. The snippet has to be re-sourced on each
shell start (it is one line). Bumping clap past 4.6 means re-checking the
completion surface by hand.
