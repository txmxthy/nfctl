# 0003 — Talk to the daemon over HTTP/JSON through a port-forward

Status: accepted · 2026-09-05

## Context
The daemon exposes gRPC and a grpc-gateway JSON API on the same port (4327) behind a
self-signed certificate with no authentication. It is not reachable from a laptop
without a port-forward. Numaflow's own REST client disables certificate verification.

## Decision
HTTP/1.1 + JSON via a hyper client whose connector opens a `kube` port-forward to the
daemon pod and wraps it in TLS with verification disabled. No tonic, no checked-in
protos. `--daemon-url` bypasses the forward for in-cluster or pre-forwarded use;
direct HTTPS verifies the server name and certificate against the platform trust store.
Reconnection lives in the connector: a dead forward is re-established on the next
request, bounded by a small retry count.

## Consequences
Hand-written DTOs must follow ProtoJSON rules (64-bit integers arrive as strings,
unset fields are absent). reqwest is not usable because it has no custom-connector
hook. The JSON layer is testable against a plain HTTP mock.
