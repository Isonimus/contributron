---
id: '0002'
title: "Rust for the core, Node only for the document toolchain"
type: architecture
status: accepted
date: 2026-11-23
supersedes: []
superseded_by: []
---

# ADR-0002 — Rust for the core, Node only for the document toolchain

## Context

The core of this product is a long-running daemon: concurrent polling of remote APIs,
durable state across restarts, supervision of child processes that run for minutes, and a
policy check that must hold under concurrency. That is a workload where a strict type
system and a compiler that refuses ambiguity pay for themselves repeatedly.

TypeScript on Node was proposed first, on the argument that iteration speed dominates cost
in an iterative development workflow and that a faster edit-to-verdict cycle therefore
outweighs the stronger guarantees of Rust.

That argument contains a measurable claim, so it was measured rather than asserted.
`scripts/stack-cycle-probe.mjs` scaffolds an equivalent trivial library in each stack and
times setup, a cold check-and-test, and a warm cycle after a one-line edit.

**The measurement did not support the claim, and is itself not trustworthy.** Two
independent defects:

- The host carried a load average of 19.34 during the run. Identical work took 2605 ms and
  then 6818 ms across two runs, a 2.6x swing attributable entirely to contention.
- The realistic-dependency stage for Rust never compiled. `octocrab` requires transitive
  dependencies needing rustc 1.88, and the active toolchain was 1.86.0, so the recorded
  178 ms is a dependency-resolution failure rather than a build.

What can be said is narrower: nothing measured supports the claim that TypeScript iterates
faster, and at trivial scale the opposite appeared, by roughly an order of magnitude. No
number from that run is cited as evidence here, and a ledger item holds the re-measurement
open.

## Decision

The core is written in Rust, as a Cargo workspace.

Node is present in this repository for exactly one purpose: running the stele document
toolchain, whose linter, index builder and hooks are `.mjs`. A `package.json` exists so
that stele rule R11 has a surface to check verification scripts against. It carries no
application code, no application dependency, and no build step for anything in `src`.

The stack decision is **not** justified by compile-time performance, in either direction.
It rests on fitness for a concurrent supervisory daemon and on the value of a strict
compiler as an automated check, which matters more here than on a typical project because
the development method leans on executable checks as its stopping condition.

## Consequences

Two toolchains live in the repository. The boundary is stated above and is narrow enough to
audit: any application dependency appearing in `package.json` is a defect.

Generation of Rust is slower and more error-prone than generation of TypeScript, so more
time is spent per unit of code. This is accepted in exchange for a compiler that rejects a
larger class of defect without a test having to be written for it.

The GitHub client will be `octocrab` or equivalent, which pins a minimum toolchain
substantially newer than the 1.86.0 found installed. The toolchain requirement is therefore
explicit in the repository rather than assumed from the developer's machine.

The global quality bar's prohibition on `any` has no direct Rust analogue. Its equivalents
are a deny-level `clippy` configuration and a prohibition on `unwrap` and `expect` outside
tests and startup, so that the "fail loud" rule is enforced by the compiler and linter
rather than by review. `docs/quality-bar.md` is adapted accordingly, as stele:ADR-0024
intends.

If the cost of generating Rust proves to dominate development time in practice, this
decision is superseded, and the superseding record cites measured cycle counts rather than
the impression of slowness.
