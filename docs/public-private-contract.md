# Public and Private Contract

This document defines the intended boundary between the public HStack repository and the private backend repository.

It exists to keep the open-source version self-contained and useful while allowing the private product to evolve further in areas that require infrastructure cost, privileged backend logic, or commercial differentiation.

## Purpose

HStack should have:

- a self-contained public product that is coherent on its own
- a shared contract layer for overlap between public and private systems
- a private backend that can grow beyond the public version where that is operationally or commercially necessary

The goal is not feature parity.

The goal is a clean architecture where the open version remains real and usable, and the private version can add more capability without distorting the public codebase.

## Repo Roles

### hstack-core

`hstack-core` is the shared contract and utility layer. It now lives in the [`humanstack-agent`](https://github.com/HStack-Org/humanstack-agent) repository alongside `hstack-agent`.

It should contain:

- shared DTOs and API models
- shared domain models
- shared validation rules
- shared protocol-level enums and types
- utilities that do not depend on private infrastructure assumptions

It should not become a container for private backend policy or implementation details.

### hstack-open

The public repository should remain:

- self-contained
- open-source friendly
- capable on the client side
- backed by a minimal but honest server implementation

The open server is allowed to be simpler than the private backend.

That is intentional.

### hstack-server-private

The private backend is where more complex capabilities may live, including but not limited to:

- advanced authentication flows
- managed cloud concerns
- privileged orchestration
- VM or execution environment management
- infrastructure-heavy integrations
- operational features that finance continued project evolution

Private-only capabilities are normal and expected.

## Contract Rules

When deciding where code belongs, use these rules.

### Put code in hstack-core only if it is protocol or domain overlap

Good candidates:

- user-facing shared data structures
- synchronization message types
- ticket and integration models that both public and private systems need to understand

Bad candidates:

- provider-specific backend workflows
- private token lifecycle handling
- infra orchestration logic
- private scheduling or worker assumptions

### Keep public implementations minimal but real

The public version should not be a fake shell around the private product.

If a capability exists publicly, the public implementation should stand on its own even if it is reduced in scope.

### Allow private capability asymmetry

The private version does not need to match the public version feature-for-feature.

It is acceptable and expected that the private product will develop capabilities that the lite or open version does not have.

### Do not leak private assumptions into public interfaces without necessity

If a field, enum variant, workflow, or abstraction exists only because the private backend needs it, it should stay out of the shared surface unless the public client genuinely needs to understand it.

### Prefer additive private extensions over shared coupling

The private backend may implement more providers, richer flows, or deeper lifecycle management.

That should usually appear as a private implementation extension, not as pressure to widen `hstack-core` prematurely.

## Practical Review Questions

Before adding a shared model or API field, ask:

1. Does the public product need to understand this concept?
2. Is this domain overlap, or just private implementation detail?
3. Would adding this make the open version less self-contained?
4. Can the private backend keep this as an internal extension instead?

If the answer to question 2 is "private implementation detail", it should not go into `hstack-core`.

## Guidance For Future Agents

When modifying shared models or public server behavior:

- review this document first
- preserve the self-contained nature of the public repo
- keep `hstack-core` focused on stable overlap between public and private systems
- do not assume the public and private products must remain feature-equivalent

If in doubt, prefer a smaller shared contract and a clearer private extension point.

## Licensing Alignment

The licensing model follows the same architectural split:

- product code in the public repo uses GPL-3.0-only
- `hstack-core` uses MPL-2.0

See [docs/licensing.md](licensing.md) for the concrete license layout and rationale.
