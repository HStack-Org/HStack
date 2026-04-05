# Agent Workspace And Viewport Specification

This document defines the formal workspace model for `hstack-agent`.
It is the blueprint for context management, app management, viewport allocation,
and kernel guarantees.

This document is intentionally architectural and normative.
It is not a code sketch, not an implementation note, and not prompt advice.

The purpose of this specification is to replace ad hoc context assembly with a
bounded, formally managed workspace model.

## Status

This document is a design specification.

- It defines the target model.
- It does not claim that the current code fully implements this model.
- If the implementation and this document diverge, this document should be treated
  as the intended design unless explicitly superseded.

## Design Goals

The workspace model exists to satisfy the following goals:

1. The agent must operate within a fixed, bounded provider context budget.
2. The agent must never lose required short-term task continuity by accident.
3. The agent must never lose awareness of near or urgent HStack events by accident.
4. The agent must be able to shape the non-kernel portion of its own workspace.
5. The non-kernel portion of workspace must behave like userland screenspace, not
   like an opaque append-only memory dump.
6. The harness must not silently summarize, compress, or semantically rewrite
   userland content behind the agent's back.
7. Workspace composition must be formally explainable and solver-based from the outset.

## Non-Goals

The following are explicitly out of scope for this model:

1. Hidden semantic summarization of userland state.
2. Implicit "smart truncation" that changes meaning without an explicit userland action.
3. Treating every tool output as permanently mounted context.
4. Treating raw provider context as identical to the full workspace state.

## Core Principle

The agent does not possess persistent memory.
The system therefore provides a persistent workspace external to the model.

The provider context seen by the model on any step is only a bounded projection
of that workspace.

The workspace is divided into:

1. kernel regions
2. app userland regions
3. mounted viewports over those regions

## Terminology

### Workspace

The persistent structured environment in which the agent operates.
The workspace contains kernel state, app state, and app metadata.

### Context Budget

The maximum amount of provider-visible context that may be mounted for one turn.
This must be bounded conservatively below the provider's hard maximum.

### Kernel

The non-optional, harness-owned part of the workspace.
Kernel regions are always conceptually present and have stronger guarantees than apps.

### App

A stateful userland information space with explicit operations, lifecycle, and
viewport semantics.

Examples include:

- scratchpad
- websearch
- future retrieval app
- future planning app

A tool is not identical to an app.
A tool is an action surface that may operate on or through an app.

### Viewport

The mounted slice of a kernel region or app that is actually visible in provider context.

### Dock

The workspace control surface for app lifecycle, app focus, and mounted viewport state.
The dock is kernel-adjacent and has a guaranteed visible footprint.

### Open App

An app whose state is live in the workspace and is listed in the dock.

### Mounted App

An app with at least one viewport currently included in provider-visible context.

### Focused App

The app currently receiving primary userland attention from the agent.

### Pinned App

An app or viewport that the allocator must preserve unless doing so would violate
kernel constraints or explicit higher-order rules.

## Workspace Model

Let:

- `W_t` be the full workspace state at time `t`
- `K_prompt` be the core prompt kernel
- `K_short(t)` be the short-term memory kernel at time `t`
- `K_event(t)` be the near-event kernel at time `t`
- `K_dock(t)` be the dock kernel at time `t`
- `A_i(t)` be the state of app `i` at time `t`
- `V_x(t)` be the mounted viewport for region or app `x` at time `t`
- `B_max` be the provider's hard context budget
- `B_use` be the internal usable context budget, where `B_use < B_max`

Then the provider-visible context at time `t` is:

$$
C_t = V_{prompt}(t) \cup V_{short}(t) \cup V_{event}(t) \cup V_{dock}(t) \cup \bigcup_i V_i(t)
$$

subject to:

$$
\operatorname{cost}(C_t) \le B_{use} < B_{max}
$$

Where:

- `V_prompt(t)` is derived from `K_prompt`
- `V_short(t)` is derived from `K_short(t)`
- `V_event(t)` is derived from `K_event(t)`
- `V_dock(t)` is derived from `K_dock(t)`
- each `V_i(t)` is a mounted viewport over app `A_i(t)`

## Kernel Regions

The kernel consists of four regions.

### 1. Core Prompt Kernel

Purpose:

- define operating contract
- define action semantics
- define available tool and app capabilities
- define non-negotiable runtime rules

Properties:

- immutable from ordinary userland operations
- always present
- never closable
- reserved context budget

This kernel is the normative behavioral contract of the agent.

### 2. Short-Term Memory Kernel

Purpose:

- preserve immediate task continuity
- preserve recent validated interaction state required to continue the current task

Properties:

- always present
- not closable
- reserved context budget
- protected from ordinary userland edits

Guarantee:

The short-term memory kernel must preserve enough recent validated interaction state
to continue the current task without accidental forgetting caused by workspace allocation.

Retention policy version: `v1_last_user_goal_pinned`

This baseline policy requires that the most recent user message is always mounted in the
short-term kernel, even if budget pressure forces more aggressive trimming of surrounding
recent messages. This prevents long action chains from evicting the initiating goal.

### 3. Near-Event Kernel

Purpose:

- preserve awareness of temporally near or urgent HStack items

Properties:

- always present
- not closable
- reserved context budget
- automatically refreshed from world state

Guarantee:

If an HStack item satisfies the near-event policy, it must remain represented in the
near-event kernel regardless of userland app state.

Near-event policy version: `v1_72h_or_urgent`

This baseline policy includes any item that either:

1. has a scheduled or expiry timestamp within the next 72 hours
2. is marked with urgent priority

### 4. Dock Kernel

Purpose:

- expose app lifecycle state
- expose app focus state
- expose mounted viewport state
- provide workspace navigation control

Properties:

- always present
- not closable
- guaranteed minimal viewport footprint

The dock must remain visible enough that the agent can reason about what is open,
what is mounted, and what is focused.

## Userland App Model

Everything outside the kernel is userland.

An app is a structured region with the following properties:

1. identity
2. state
3. lifecycle state
4. viewport semantics
5. action surface
6. allocation metadata

Each app must define:

- `AppId`
- whether it is installed
- whether it is open
- whether it is focused
- whether it is pinned
- candidate viewport forms
- allowed navigation/edit operations
- context cost model
- persistence semantics

## App Lifecycle

Each app exists in exactly one of the following lifecycle states:

1. installed, closed
2. open, unmounted
3. open, mounted
4. open, mounted, focused

Additional orthogonal flags may exist:

- pinned
- urgent
- background

### Lifecycle Meaning

Installed, closed:

- the app exists in the environment
- it may be opened
- it consumes no mounted context budget except dock metadata

Open, unmounted:

- the app has live state in the workspace
- it appears in the dock
- none of its content is currently mounted into provider-visible context

Open, mounted:

- the app contributes at least one viewport to provider-visible context

Open, mounted, focused:

- the app is the primary userland workspace target for the current step
- the allocator should typically privilege its viewport candidates

## Installed, Open, And Mounted Are Distinct

The following distinctions are mandatory:

1. Installed does not imply open.
2. Open does not imply mounted.
3. Mounted does not imply full app visibility.
4. Focused does not imply exclusivity.

This separation is necessary to prevent uncontrolled context growth.

## Dock Specification

The dock is the control plane for apps.

### Dock Responsibilities

The dock must expose at least:

1. installed apps relevant to the current environment
2. open apps
3. focused app
4. pinned apps
5. mounted viewports per app
6. app-level urgency or activity indicators
7. enough allocation metadata for the agent to reason about workspace composition

### Dock Guarantee

The dock has a guaranteed minimal mounted viewport.

The dock viewport must be large enough that the agent can always answer these questions:

1. Which apps are currently open?
2. Which app is focused?
3. Which apps are mounted?
4. Which apps are pinned or urgent?
5. Which app-management actions are currently available?

### Dock Is Not Ordinary Userland

The dock is exposed to the agent operationally, but it is not an ordinary app.
It is kernel-adjacent and must not be removable by normal app-management actions.

## Viewport Semantics

The allocator never mounts a full app by default.
It mounts only viewports.

Each app defines its own viewport semantics.

Examples:

- scratchpad app: contiguous text window with cursor/anchor state
- websearch app: result list window and possibly a focused result document window
- retrieval app: result cluster or focused item window

### Viewport Invariants

1. A viewport is only a slice, never the whole app unless explicitly small enough.
2. The same app may contribute multiple viewports if policy allows.
3. Viewport movement is explicit.
4. The harness must not silently replace a viewport with a semantic summary.

## No Hidden Summarization

This is a hard rule.

The harness must not silently summarize userland app content in order to save space.

If userland content is transformed into a shorter form, that transformation must itself
be represented as an explicit agent-mediated operation over userland content.

Corollary:

- truncation means reducing mounted viewport extent
- not replacing full content with hidden semantic abstraction

## Scratchpad App Requirements

The scratchpad must behave like an editable workspace document, not like an append-only log.

The scratchpad must support operations equivalent in spirit to:

1. read visible window
2. search within document
3. move up/down
4. jump to match or anchor
5. insert
6. delete
7. replace
8. patch by diff-like operation

The scratchpad is userland.
It is not a kernel guarantee.

The agent should only see a bounded slice of the scratchpad at a time, subject to
viewport allocation.

## Websearch App Requirements

The websearch capability should be modeled as an app, not merely as transient tool output.

The websearch app must support at least:

1. query history
2. result-set state
3. focused result selection
4. viewport mounting over search results
5. explicit open/close behavior

Websearch output is therefore part of workspace userland rather than being implicitly
stuffed into context forever.

## App Management Actions

The workspace model assumes explicit app-management actions.

At minimum, the system should eventually support actions equivalent to:

1. `open_app(app_id)`
2. `close_app(app_id)`
3. `focus_app(app_id)`
4. `pin_app(app_id)`
5. `unpin_app(app_id)`
6. `list_open_apps()`
7. `inspect_app(app_id)`
8. `scroll_app(app_id, delta)`
9. `search_app(app_id, query)`
10. `jump_app(app_id, anchor)`

These are not implementation commitments for exact API names.
They define the required capability surface.

## Allocation Problem

After kernel reservation, the remaining workspace budget must be allocated across apps.

Let:

- `B_prompt`, `B_short`, `B_event`, `B_dock` be reserved kernel budgets
- `B_apps = B_use - (B_prompt + B_short + B_event + B_dock)`

Then the allocator must choose app viewports such that:

$$
\sum_i \operatorname{cost}(V_i) \le B_{apps}
$$

### Allocation Is A Planning Problem

App allocation should be treated as a planning problem over candidate viewports.

Each app may produce multiple candidate viewport choices with different:

- costs
- utility values
- focus alignment
- urgency contributions

The allocator then selects a feasible set under budget.

## Priority Model

App priority must be formally expressible.

Priority should not be a single informal adjective.
It should be derived from factors such as:

1. app class
2. focus status
3. pinned status
4. urgency
5. recency of use
6. relevance to current task
7. marginal utility per unit of context cost

### Priority Classes

The first formal model should distinguish at least these classes:

1. kernel mandatory
2. foreground app
3. supporting app
4. background app

Kernel mandatory regions always win.

Foreground apps should generally outrank supporting apps.
Supporting apps should generally outrank background apps.

## Solver Formulation

The allocation problem must be expressed as an optimization problem under a hard budget.

Let `V_i^j` be candidate viewport `j` for app `i`.
Let `u_i^j(t)` be the utility of that viewport at time `t`.
Let `c_i^j(t)` be its cost.

Then a general formulation is:

$$
\max \sum_{i,j} x_i^j u_i^j(t)
$$

subject to:

$$
\sum_{i,j} x_i^j c_i^j(t) \le B_{apps}
$$

with additional constraints such as:

- exclusivity where needed
- required inclusion for pinned apps
- required inclusion for focused app minimum viewport
- dock visibility guarantee

where `x_i^j \in \{0,1\}` or a similarly constrained selector domain.

This specification does not permit a heuristic allocator as the normative baseline.
The intended design is solver-based from the beginning.

Any temporary implementation shortcut that does not solve the declared allocation
problem should be treated as non-conformant with this specification unless explicitly
documented as a provisional deviation.

## Solver Requirements

The allocator must solve, or soundly approximate through a formally defined optimization
procedure, the declared workspace allocation problem.

At minimum, the solver must respect all hard constraints:

1. kernel regions are always included
2. dock minimum visibility is always preserved
3. required kernel guarantees are never evicted
4. pinned viewports are preserved when declared mandatory by policy
5. total mounted cost never exceeds `B_use`

The solver must not be replaced by an informal greedy or priority-only heuristic and
still be described as implementing this specification.

If approximation is ever used, the approximation strategy itself must be formalized as
part of the specification and justified against the exact objective and constraints.

## Capacity Discipline

The system must use an internal usable budget smaller than the provider's hard limit.

This slack exists to cover:

1. provider formatting overhead
2. tool schema overhead
3. model-specific tokenization variance
4. response headroom

The workspace allocator must operate against the internal usable budget, not the hard maximum.

## Non-Interference Rules

The following rules are mandatory:

1. Userland app operations must not violate kernel guarantees.
2. Closing or resizing userland apps must not evict kernel regions.
3. App allocation must not make the dock invisible.
4. App allocation must not erase short-term or near-event guarantees.
5. The harness must not rewrite userland semantics without explicit userland operations.

## Persistence Model

The workspace exists outside the LLM.
The provider sees only mounted viewports.

Therefore:

1. full app state may persist while unmounted
2. app closure does not necessarily imply destruction unless policy says so
3. mounted context is always a projection, never the full workspace itself

This is necessary because the model has no intrinsic persistent memory.

## Rust Encoding Requirements

This specification should be realized in Rust in a way that leverages the type system
and compiler guarantees as much as possible.

The implementation should not rely primarily on comments, conventions, or runtime
discipline for invariants that can be encoded in types.

### General Rule

Whenever an invariant from this specification can be made statically checkable in Rust,
it should be encoded so that invalid states are unrepresentable or at least difficult
to construct.

### Required Direction

The implementation should prefer:

1. enums over ad hoc string categories
2. dedicated structs and newtypes over unstructured maps
3. closed state machines over boolean flag soup
4. explicit region and app identifiers over raw strings when practical
5. typed action payloads over generic unvalidated blobs once data crosses the external boundary

### Boundary Rule

External boundaries may still require flexible representations such as JSON.
That is acceptable only at the boundary.

After decoding external input, the system should move into typed internal representations
as early as possible.

In particular:

- provider/tool payloads may arrive as JSON
- app/userland documents may persist as serialized data
- but internal workspace allocation, lifecycle state, kernel regions, and viewport decisions
  should be represented with typed Rust structures

### Invalid States Should Be Unrepresentable

Examples of desired direction include:

1. distinguishing installed, open, mounted, and focused app states through an explicit
   state model rather than loosely correlated booleans
2. representing kernel regions with dedicated types instead of generic tags
3. representing solver outputs as validated allocation plans rather than arbitrary lists
4. representing anomaly kinds, app kinds, and region kinds with closed enums
5. representing viewport bounds and budgeted slices with validated types rather than raw
   unchecked offsets

### Strong Preference Against Stringly-Typed Core Logic

Core workspace logic should not depend on string comparisons for essential semantics
when a finite enum or newtype can carry the same meaning.

This applies especially to:

1. app lifecycle state
2. kernel region identity
3. viewport class
4. allocation priority class
5. solver constraint category
6. dock state categories

### Type-Level Separation

The implementation should strongly separate:

1. kernel data from userland app data
2. installed apps from open apps
3. open apps from mounted viewports
4. requested allocations from validated allocations
5. external serialized forms from internal validated forms

This separation should appear in Rust types, not only in prose.

### Compiler-Assisted Verification

The Rust compiler should be used as a first line of architectural verification.

Desired consequences include:

1. illegal transitions fail to compile where possible
2. missing match arms expose incomplete handling of workspace states
3. region and app distinctions are checked structurally rather than remembered informally
4. future refactors break loudly when invariants are violated

### Runtime Checks Still Exist

Not every invariant can be discharged statically.
Budget calculations, solver outputs, provider token costs, and world-time event predicates
still require runtime validation.

However, runtime checks should sit on top of a strongly typed model, not replace it.

### Conformance Expectation

An implementation should be judged better, not worse, when it moves architectural rules
from documentation into Rust types.

If a rule remains only in prose when it could reasonably be enforced by the compiler,
that should be treated as a design gap and justified explicitly.

## Current Status Relative To This Specification

The current implementation now encodes the baseline design described above for the built-in
workspace apps.

In particular, it now provides:

1. persistent mounted-app state stored in workspace state and refreshed from validated allocation plans
2. an explicit short-term retention policy that pins the latest user message in the kernel
3. an explicit near-event policy based on 72-hour temporal proximity or urgent priority
4. dock visibility in both provider context and the CLI surface
5. typed app lifecycle, dock state, viewport allocation, and workspace mutation paths in Rust

Future work can still broaden the app surface, but those extensions are no longer blockers for
the baseline workspace model.

## Proof Obligations For Future Implementation

Any implementation claiming conformance with this specification should eventually
demonstrate at least the following:

1. Kernel visibility is preserved under all admissible app operations.
2. Short-term memory guarantee cannot be accidentally evicted by userland growth.
3. Near-event memory guarantee cannot be accidentally evicted by userland growth.
4. The dock remains sufficiently visible for app management.
5. Open apps can exist without being mounted.
6. Mounted apps can be reallocated without semantic rewriting.
7. Userland content is not silently summarized by the harness.
8. Allocation decisions are explainable from declared policy.

## Implementation Order Recommendation

The following order is recommended:

1. formalize kernel state and kernel budgets
2. formalize dock state and app lifecycle model
3. formalize viewport representation
4. formalize app state model for scratchpad and websearch
5. translate core invariants into Rust types and state models wherever practical
6. formalize the exact optimization objective and hard constraints
7. build the solver-backed allocator directly

## Normative Summary

This specification defines the agent workspace as a bounded operating environment.

- Kernels are guaranteed, protected, and always conceptually present.
- Apps are userland spaces with lifecycle and viewports.
- The dock is a guaranteed control surface for app management.
- Provider context is a mounted projection, not the workspace itself.
- Userland is never silently summarized by the harness.
- Allocation is a formal planning problem under a hard budget.

Any future context-management implementation should be judged against those principles.
