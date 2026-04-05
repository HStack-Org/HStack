# Formal Verification

This document defines the machine-checkable verification entrypoint for the `hstack-agent`
harness semantics.

## Scope

The formal model currently covers these semantic obligations:

1. only `identity` may terminate a user-visible turn
2. `follow_up` is non-terminal
3. forced-terminal mode admits only `identity`
4. multiple provider tool calls in a single step are structural anomalies, not progress
5. unknown tools are structural anomalies, not progress
6. turns with no tool call are structural anomalies, not progress
7. malformed arguments are structural anomalies, not progress
8. tool execution failure is a structural anomaly, not progress
9. assistant narration has no semantic force by itself
10. assistant narration does not change the decoded outcome of a fixed tool path
11. terminalization after non-progress still produces a terminal reply even when the forced turn is invalid

These properties are encoded in [crates/hstack-agent/src/formal.rs](crates/hstack-agent/src/formal.rs).

The formal module is intentionally small and pure. It is not a second runtime.
It is a verifier-oriented semantic skeleton for the protocol rules that must not regress.

## Local Unit Checks

The pure model is also covered by ordinary tests:

```bash
cargo test -p hstack-agent formal::tests
```

## Kani Setup

Install the verifier tooling with the same steps used in CI:

```bash
cargo install --locked kani-verifier
cargo kani setup
```

## Kani Run

Run the harness-semantic proofs with:

```bash
cargo kani -p hstack-agent --lib
```

This will pick up the proofs under `#[cfg(kani)]` in [crates/hstack-agent/src/formal.rs](crates/hstack-agent/src/formal.rs).

The repository CI workflow runs this same command in [.github/workflows/kani.yml](.github/workflows/kani.yml).

## Required Outcome

Any change to terminal semantics, decode admissibility, follow-up behavior, or forced-terminal admissibility must keep the Kani proofs green.

If a future refactor changes the semantic model, update both:

1. the runtime implementation
2. the formal model and its proofs

Do not update only one of them.

## Next Formalization Targets

The current formal model is still intentionally smaller than the full runtime. The next targets to lift into verifier-friendly pure code are:

1. allowed-tool-set filtering as an explicit mathematical input to decode
2. compound-action semantics and the proof that only embedded `Stop` yields termination
3. correspondence lemmas between the pure model and the Rust runtime decoder
4. correspondence lemmas for deterministic host fallback and the concrete runtime stop path
