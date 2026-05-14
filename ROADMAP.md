# HStack Open Roadmap

This roadmap tracks work that belongs in the public repository:

- the open product surface
- `hstack-core` and `hstack-agent` from the [`humanstack-agent`](https://github.com/HStack-Org/humanstack-agent) repository as the shared contract and agent layers
- the public desktop app and lite server

Private backend work is intentionally tracked separately in the private repository roadmap.

## Scope

This roadmap includes:

- shared domain and contract work that lives in `hstack-core` (in the `humanstack-agent` repo)
- public app experience and rendering work
- lite-server behavior that keeps the open product self-contained

This roadmap does not include:

- managed-cloud auth rollout
- server-side external integration infrastructure
- private-only orchestration or execution-environment work

## Principles

- Prefer strict data models over heuristic parsing.
- Prefer discriminator-driven decoding over shape-based guessing.
- Prefer shared abstractions when they reduce long-term maintenance cost.
- Prefer staged delivery for UI and maps instead of jumping directly to the heaviest integration.
- Planner behavior should be enforced by contracts and validation, not only prompt wording.
- Keep the public product coherent on its own, even when the private product grows faster.

## Current Priorities

Progress note:

- Roadmap items can be checked off as they ship.
- We should only mark an item complete when the data model, behavior, and UI are all in place for the public product.

### 1. Planner Hardening

Goal: make proactive planning reliable, explicit, and testable in the shared domain layer.

Planned work:

- Add transcript-based planner regression tests.
- Cover dependency-aware planning scenarios.
- Ensure impacted tickets are detected and updated when anchor events move.
- Tighten planner validation around grounded facts and required follow-up actions.
- Continue reducing cases where the planner can produce internally inconsistent actions.

Key scenarios to test:

- Moving a birthday event should consider tickets anchored to that birthday.
- Blocking commitments should reschedule dependent tasks and optionally create missing commitments.
- The planner should not fabricate names, provenance, or unsupported dependencies.

Implementation checklist:

- [x] Add planner-output regression tests for JSON extraction and dependency-aware plans.
- [x] Reject planner actions that are not backed by grounded facts.
- [x] Require dependency follow-up actions to align with `action_required` flags.
- [x] Reject duplicate dependency entries and malformed commitment records.

### 2. Shared Scheduling Model

Goal: unify scheduling primitives across ticket types without forcing identical semantics onto every type.

Direction:

- Introduce a shared schedule abstraction built around DTSTART and optional RRULE.
- Reduce duplicated scheduling fields and logic across TASK, HABIT, EVENT, and COUNTDOWN.
- Keep type-specific semantics on top of the shared scheduling layer.
- Preserve clear invariants rather than collapsing ticket types into one generic schedule shape.

Expected outcomes:

- Cleaner Rust helpers and traits around time-bearing tickets.
- Less duplicated scheduling code.
- Easier long-term maintenance and evolution.

Implementation checklist:

- [x] Introduce a shared schedule abstraction for TASK, HABIT, and EVENT in the public ticket model.
- [x] Keep type-specific semantics layered on top of the shared schedule shape.
- [x] Refactor public app schedule parsing and sorting to use a shared schedule extractor.
- [x] Add shared-core tests covering schedule extraction and updates.

### 3. Event And Commute Location Model

Goal: allow events and commute-related logic to use structured location information in a shared, typed way.

Approved direction:

- Add an optional structured `Location` object to EVENT.
- Use a `location_type` enum instead of sentinel strings.
- Support different location representations in a typed way.

First-pass location types:

- `Coordinates`
- `AddressText`
- `PlaceId`
- `CurrentPosition`

Design constraints:

- Avoid magic strings like `current_location` in raw payload fields.
- Keep location optional on EVENT.
- Separate what the user said from resolved place metadata where useful.

Implementation checklist:

- [x] Add a typed `Location` model with explicit variants for address text, coordinates, place IDs, and current position.
- [x] Extend EVENT and COMMUTE payloads to carry structured locations.
- [x] Enforce strict location normalization and reject inconsistent structured/text inputs.
- [x] Render structured location details in the public ticket UI.
- [x] Add shared-core and app-level tests for location decoding and validation.

### 4. Commute Inference From Events

Goal: if an event has a time and a location, HStack can reason about whether a commute ticket is needed.

Planned behavior:

- Use structured locations for commute origin and destination.
- Allow planner logic to infer commute needs from scheduled events.
- Use `CurrentPosition` as an explicit location variant where appropriate.
- Only create or update commute tickets when routing inputs are concrete enough.

Implementation checklist:

- [x] Extend COMMUTE payloads with structured origin and destination locations.
- [x] Anchor inferred commute tickets to their source events with explicit related IDs.
- [x] Infer companion commute tickets deterministically from scheduled events with structured destinations.
- [x] Update or remove inferred commute tickets when the source event changes or no longer qualifies.
- [x] Add app-level tests for strict location validation and commute inference.

## Public App Roadmap

### 5. Rich Ticket Rendering

Goal: render more of the information tickets already contain instead of reducing them to title plus a few tags.

Current progress:

- Schedule ordering was improved.
- Expanded cards now show more schedule and note information.

Next steps:

- Design type-specific ticket layouts instead of relying mostly on a shared generic card.
- Define a clear rendering matrix for TASK, HABIT, EVENT, COMMUTE, and COUNTDOWN.
- Surface more structured data in intentional layouts rather than generic detail rows only.
- Improve visual hierarchy for schedule, notes, duration, and dependency-relevant context.

Type-specific rendering goals:

- TASK: schedule, notes, duration, dependency context.
- HABIT: recurrence-first presentation, cadence, completion context.
- EVENT: time, duration, location, related commute context.
- COMMUTE: route summary, timing, transport details, map actions.
- COUNTDOWN: live urgency, timer, linked context.

Implementation checklist:

- [x] Add type-specific highlight panels instead of relying only on generic detail rows.
- [x] Render structured event location and commute context in dedicated sections.
- [x] Improve visual hierarchy for schedule, duration, and status across ticket types.
- [x] Keep countdown and commute cards visibly richer than the generic baseline.

### 6. Minimal Map Experience

Goal: improve commute usefulness without immediately taking on the complexity of embedded maps.

Approved first step:

- Add deep links to external map providers.
- Show richer route summary and destination context inside commute tickets.

Possible first integrations:

- Open in Google Maps
- Open in Apple Maps

Deferred for later:

- Embedded map previews
- Static route thumbnails
- Live map surfaces inside the app

Implementation checklist:

- [x] Add deep links to Google Maps from commute tickets.
- [x] Add deep links to Apple Maps from commute tickets.
- [x] Keep the first map experience external-link based instead of embedding map surfaces.

### 7. Ticket Status And Priority

Goal: add optional status and priority fields across ticket types so the public product reaches a more realistic project-management baseline and is ready for future integration parity with project-management software.

Approved direction:

- Add optional `status` and `priority` fields to tickets.
- Keep both fields optional rather than mandatory.
- Render them as tags in the same spirit as schedule and type tags.
- Keep the public model generic enough that future integrations can map external project-management states onto it.

First-pass status direction by ticket type:

- TASK: use familiar project-management statuses such as backlog, todo, in_progress, blocked, done, cancelled.
- EVENT: support statuses that reflect intent or attendance value such as mandatory, optional, nice_to_have, cancelled.
- HABIT: support statuses that reflect commitment level or current state such as active, paused, optional, archived.
- COMMUTE and COUNTDOWN: optional status can exist, but task, event, and habit coverage are the first priority.

First-pass priority direction:

- Support a shared optional priority indicator across ticket types.
- Keep the first pass intentionally simple, for example low, medium, high, urgent.
- Allow different ticket types to use the same priority field even if the semantics differ slightly.

Design constraints:

- Do not force every ticket to have status or priority.
- Avoid hard-coding a model that only works for one external tool.
- Preserve room for future integration mapping to GitHub, Jira, Linear, or similar systems.
- Treat status and priority as product-level first-class fields, not just note text.
- Show them in the ticket UI as lightweight tags rather than burying them in expanded details only.

Implementation checklist:

- [x] Define optional shared status and priority fields in the public ticket model.
- [x] Add first-pass status enums or validated values for TASK, EVENT, and HABIT.
- [x] Add a shared optional priority indicator for public tickets.
- [x] Ensure sync and serialization preserve the new optional fields.
- [x] Render status and priority as tags on ticket cards.
- [x] Keep legacy tickets working when the new fields are absent.
- [x] Add tests for model decoding, sync behavior, and UI rendering expectations where practical.

## Deferred / Later Phase

### 8. Saved Locations / Reference Locations

Goal: support reusable user places like home, office, gym, and similar anchors.

Implementation checklist:

- [x] Add a typed saved-location collection to public user settings.
- [x] Reuse the shared location model shape for saved location records.
- [x] Add strict add and remove controls in the public settings UI.
- [x] Keep the first pass focused on explicit address-text saved places.

### 9. Embedded Maps

Goal: add lightweight in-app map previews for location-bearing tickets without pulling in a heavy private-only map stack.

Implementation checklist:

- [x] Add embedded map previews for EVENT tickets with structured locations.
- [x] Add embedded map previews for COMMUTE tickets using strict origin and destination text.
- [x] Keep the embed layer lightweight and external-provider based instead of introducing a heavy SDK.
- [x] Reuse the existing location rendering pipeline so maps stay aligned with strict ticket data.

## Suggested Implementation Order

1. Planner regression suite and contract hardening
2. Shared scheduling abstraction design
3. Structured `Location` model for EVENT and COMMUTE
4. Commute inference from scheduled events with location
5. Rich ticket renderer by type
6. Minimal map deep-link experience
7. Ticket status and priority across ticket types
8. Saved/reference locations
9. Embedded map previews or live maps

## Notes

- This roadmap is meant to capture approved directions, not lock implementation details prematurely.
- Architecture changes should be validated before broad UI expansion.
- When in doubt, prefer fewer concepts with stronger invariants over flexible but ambiguous payloads.
- Managed-cloud auth, integration infrastructure, and private-only capability expansion are intentionally tracked outside the public repo.
