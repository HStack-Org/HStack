# HStack Agent Rules

These rules are mandatory for human contributors and AI coding agents working in this repository.

## Core Standards

1. Fail closed.
   Missing config, invalid auth, malformed payloads, or unavailable remote dependencies must return an explicit error. Do not add insecure defaults, silent fallbacks, or development backdoors.

2. `unwrap` and `expect` are forbidden in repository Rust source.
   The crates in this repo deny `clippy::unwrap_used` and `clippy::expect_used`. Keep new code compatible with that policy, including tests.

3. The browser must not own sync transport.
   Websocket lifecycle, authentication, reconnect logic, and pending-action flushing belong in the Tauri Rust layer. The frontend is for rendering state and sending intent through commands.

4. Sync state is Rust-owned.
   Canonical local sync state lives in the Tauri stores (`base_state.json` and `pending_actions.json`). Do not introduce parallel browser-only task or sync-history stores.

5. Sync must be authenticated and user-scoped.
   Never trust a user ID from the UI by itself. All remote sync operations must be tied to authenticated session data and scoped server-side.

## Architectural Guardrails

1. Frontend responsibilities:
   Render tasks, react to Tauri events, invoke Tauri commands, collect user input.

2. Tauri Rust responsibilities:
   Persist base state and pending actions, own websocket/runtime transport, reconcile server state, emit task/status events to the UI.

3. Server responsibilities:
   Authenticate sync sessions, scope all mutations to the authenticated user, acknowledge only committed writes, and emit explicit remote state change notifications.

## Agent Harness Invariants

These rules define the semantics of the agentic harness. Treat them as protocol invariants, not prompt suggestions.

1. The harness is action-based.
   A model turn is only meaningful if it decodes to a valid `AgentAction`. Free-form assistant prose is not, by itself, a state transition.

2. The only user-facing terminal answer is a terminal action.
   In the current design, that means the model must call the `identity` tool, which returns `AgentAction::Stop(answer)`.

3. Bare assistant text is not completion.
   If the provider returns natural-language assistant content without a valid tool call that maps to an action, the harness must not treat that as progress, completion, or a reply to surface to the user.

4. Harness/runtime state is structural, not conversational.
   Internal anomalies such as invalid tool names, malformed arguments, empty model output, iteration limits, or provider protocol mismatches may be recorded in technical/debug state, but the harness must not impersonate the assistant by fabricating fallback prose.

5. Working memory is not the user transcript.
   Working memory may contain raw provider artifacts, tool traces, and technical noise for debugging, but only validated conversational actions belong in the user-visible conversation.

6. Progress means a validated transition.
   Advancing the loop requires an action that is valid under the transition algebra, not a heuristic interpretation that "the model probably meant well enough".

7. If no valid action exists, fail structurally.
   The correct outcome is an explicit runtime/protocol anomaly or bounded halt, not heuristic guesswork.

Minimal formal sketch:

- Let `C_n` be the current harness state.
- Let `m_n` be the raw provider output for step `n`.
- Let `decode(m_n)` produce either a valid `AgentAction` or a protocol/runtime anomaly.
- Let `apply(a, C_n) = C_n+1` for valid actions.

Then the loop is:

1. construct context from `C_n`
2. obtain `m_n` from the provider
3. compute `decode(m_n)`
4. if `decode(m_n) = a`, apply `a`
5. otherwise record the anomaly structurally and do not treat `m_n` as conversational progress

Corollary: raw assistant prose has no semantic effect unless it is carried by a valid terminal action.

## Review Checklist

Before merging sync-related work, verify all of the following:

1. No browser `new WebSocket(...)` exists in application code.
2. No Rust `unwrap` / `expect` was introduced.
3. Remote sync still works with reconnects and explicit error reporting.
4. Pending actions remain durable across app restarts.
5. The UI updates through Tauri commands/events, not browser-local shadow state.

## If You Need To Bend A Rule

Stop and document the reason in the PR or commit discussion first. Do not quietly add exceptions.
