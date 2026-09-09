# 0008 — Keep credential-plugin noise off the completion path

Status: accepted · 2026-09-09

## Context
A kubeconfig context can authenticate through an exec credential plugin. kube
runs that plugin as a child process and, unless the context says
`interactiveMode: Never`, gives it our own stdin and stderr. When the plugin
cannot refresh non-interactively it prints an explanation and exits non-zero,
so a Tab press — which shells out to `nfctl` and is meant to print only
candidates — dumps a page of the plugin's text over the prompt line. Our error
handling cannot catch it: the bytes are written by a child straight to the
terminal and never pass through this process, and the 1.5 s budget on the
lookup only stops us waiting for a child that has already written.

That raised a design question: should `nfctl` grow a cloud-provider adapter
that checks whether the user is authenticated, starting with one provider and
adding others later?

## Decision
No provider adapter. The exec-credential mechanism in kubeconfig *is* the
provider-neutral adapter: every provider already ships a plugin behind one
contract, and a checker of ours would have to know each provider's CLI, its
token cache and its error strings — knowledge that lives outside the repo and
dates on someone else's release schedule. It would also be wrong for the
contexts that use no plugin at all.

What the problem actually needs is narrower, and is what we implemented:

1. Silence on the completion path. `complete::live` re-points this process's
   stdin and stderr at `/dev/null` for the duration of the lookup and restores
   them on drop, so anything the plugin inherits goes nowhere and it cannot
   prompt. Provider-agnostic — it works for a plugin we have never heard of.
2. The message intact on the normal path. `nfctl ls` against the same context
   still shows the plugin's own output *and* our `nfctl: cluster: auth error:
   …`, which is where the user is told to log in again. Silence is scoped to
   the completer, which has nowhere to report an error anyway.

We considered a cached "is this context usable" probe — remembering a failed
lookup so Tab does not pay 1.5 s again — and left it out for now. It is a
per-context negative entry beside the existing catalog cache, still
provider-agnostic; nothing in this decision blocks it. A provider adapter would
only be warranted if we needed something kubeconfig cannot express, such as
naming *which* command re-authenticates a given context; the plugin's own
message already carries that.

## Consequences
Tab is quiet whatever the context's credential plugin does, on unix; other
platforms keep the old behaviour because only a descriptor redirect fixes it
and only unix has one here. Anything the completion path might have written to
stderr — clap's own error for a malformed `COMPLETE` value is outside the
guard, but a panic inside the lookup is not — is swallowed, which is the price
of the guarantee; debugging the completer means running the same command with
`COMPLETE` unset. A cold Tab against an unauthenticated context still costs the
1.5 s budget once per 30 s until the negative cache above exists.
