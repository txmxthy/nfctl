# 0005 — CLI first, TUI second

Status: accepted · 2026-09-05

## Context
The risky parts are the daemon transport and log tailing through pod churn, not
the screen. A CLI is scriptable, snapshot-testable and proves both risks early.

## Decision
Build and ship the CLI through milestone M6. The TUI (M7) reuses the same services
and follows the reference layering: one `Model` per panel, an mpsc worker for I/O,
UI-owned state rather than a shared mutex.

## Consequences
Users get value before any TUI exists. Commands that are not ready are visible stubs
that exit with code 4 and point at `docs/roadmap.md`, never hidden or silently
no-op.
