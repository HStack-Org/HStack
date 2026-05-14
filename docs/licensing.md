# Licensing Model

This repository uses a split-license model aligned with the public/private architecture.

## Summary

- The open product code is licensed under GPL-3.0-only.
- `hstack-core` (in the [`humanstack-agent`](https://github.com/HStack-Org/humanstack-agent) repository) is licensed under MPL-2.0.

## Why

The project has two goals that pull in different directions:

- keep the public product genuinely open when redistributed
- keep the shared contract layer usable across the public and private boundary without making the entire private backend inherit the same copyleft scope

GPL-3.0-only is used for the app and lite server because it protects the distributed public product as a whole.

MPL-2.0 is used for `hstack-core` because it keeps modifications to the shared files open while allowing those shared contracts and utilities to participate in a broader mixed public/private architecture.

## Scope

### GPL-3.0-only

Applies to the public product code in this repository, including:

- the Tauri application crate
- the lite server crate
- the frontend and repository-level product packaging unless a narrower license notice says otherwise

### MPL-2.0

Applies to:

- `hstack-core` in the `humanstack-agent` repository

That crate has its own local license file because it is intentionally a separate shared layer.

## Practical Rule

If you are adding or moving code:

- put shared protocol/domain overlap in `hstack-core`
- keep product-specific app/server behavior out of `hstack-core`
- do not widen the MPL surface just because the private backend has new capabilities

## Source of Truth

The authoritative license markers are:

- the repository and crate/package manifest metadata
- the root [LICENSE](../LICENSE) file for GPL-3.0-only product code
- the `humanstack-agent` repository LICENSE for the MPL-2.0 shared core

When in doubt, preserve the architectural split described in [docs/public-private-contract.md](public-private-contract.md).
