# Agent Harness Invariants

This document describes the formal behavioral contract of the `hstack-agent` harness.
It is intentionally stricter than ordinary prompt guidance. These are semantic rules
for the runtime, not style preferences for model output.

For the higher-level workspace, app, dock, and viewport model that should eventually
govern context construction, see `docs/agent-workspace-viewport-spec.md`.

## Core Model

Let:

- `C_n` be the harness state at step `n`
- `m_n` be the raw provider output at step `n`
- `decode(m_n)` be the decoder from provider output into the agent action algebra
- `apply(a, C_n) = C_n+1` be the state transition induced by a valid action `a`

Then the harness loop is:

1. construct provider context from `C_n`
2. obtain `m_n` from the provider
3. compute `decode(m_n)`
4. if `decode(m_n) = a`, apply `a`
5. otherwise record the anomaly structurally and continue or halt by policy

The critical point is that `m_n` has no semantic effect on its own.

In the Rust harness, this distinction is represented explicitly by `DecodedTurn`:

- `DecodedTurn::Action(AgentAction)` means the turn decoded to a valid transition.
- `DecodedTurn::Anomaly(DecodeAnomaly)` means the turn produced only structural/runtime anomalies.

`DecodeAnomaly` is itself closed over a finite set of `DecodeAnomalyKind` variants.
That prevents the harness from silently inventing new anomaly categories through ad hoc
string keys in the decode path.

This prevents the loop from conflating "something was emitted by the provider" with
"a valid state transition exists".

## Action-Only Semantics

The harness is action-based.

- Raw assistant prose is not a transition.
- A provider turn only matters after successful decoding into a valid `AgentAction`.
- User-visible completion requires a terminal action.
- In the current implementation, terminal completion is produced by the `identity` tool,
  which returns `AgentAction::Stop(answer)`.
- Every user query must terminate with a reply. An explicit empty reply is valid; an absent
    reply is not.
- In the ordinary success path, the runtime itself must not fabricate that reply.
- After structural non-progress, the host may apply a deterministic terminal fallback reply by
    policy rather than returning an error or looping forever.

Corollary: assistant text without a valid terminal action is not a user reply.

## What Counts As Progress

Progress means a validated transition in the action algebra.

Because the harness transition is step-based, a single provider step may decode to at most
one provider-originated tool call. Any response containing multiple tool calls is structurally
invalid: later calls are not grounded in the post-state of earlier calls because the model has
not yet observed their results.

Examples of valid progress:

- a known tool call with valid arguments that produces an action
- a validated stack mutation proposal
- a terminal `Stop(answer)` action

Examples of non-progress:

- plain assistant narration with no tool call
- malformed tool arguments
- unknown tool names
- multiple tool calls in a single provider response
- empty provider output

Non-progress events may still be recorded as technical/runtime anomalies.

## Conversation Boundary

The harness must not impersonate the assistant.

- Runtime anomalies belong in technical/debug state.
- Tool traces belong in technical/debug state.
- Raw provider artifacts may be retained for observability.
- Only validated conversational actions may become user-visible assistant output.

This keeps the user transcript separate from protocol noise.

## Fail-Closed Requirements

The harness must fail structurally rather than invent semantics.

- Missing terminal answer arguments are errors.
- Malformed tool payloads are errors.
- Unknown tools are errors.
- Empty or non-actionable assistant content is an anomaly, not completion.

The terminal `identity.answer` field must be present, but it may be an empty string when
the agent terminates with an explicit empty reply.

The runtime may continue after these anomalies if policy allows, or it may invoke
deterministic host terminalization. It must not treat provider prose itself as
implicit success.

## Proof Obligations

The following behaviors must be covered by tests:

1. Bare assistant prose does not become a terminal answer.
2. Assistant narration accompanying a tool call does not become a user-visible reply by itself.
3. Malformed tool arguments do not execute the tool.
4. The terminal answer must come from an explicit terminal action.
5. Missing `identity.answer` is rejected rather than defaulted.
6. A query that hits the max-iteration policy must still terminate with a reply.
7. If forced terminalization fails to decode a valid identity action, the host must emit the
    deterministic terminal fallback instead of returning a protocol-level failure.

Those tests live in `crates/hstack-agent/src/tests.rs` and should be kept aligned with this document.

For machine-checkable proofs of the terminal-action contract, see `crates/hstack-agent/src/formal.rs`
and `docs/formal-verification.md`.
