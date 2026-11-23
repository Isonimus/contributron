# Changelog

What each release changed for the people who use it, newest first. It is kept by hand, in the
same commit as the change it describes. The decisions behind each entry are in
[`adr/`](adr/); the open work is in [`LEDGER.md`](LEDGER.md).

## Unreleased

Nothing has been released. The repository currently contains its own governance: the
decisions the work will obey, the sequenced plan, and the checks that enforce both.

- **The decision record.** Six ADRs narrowing a multi-year platform design to a reachable
  MVP: one provider adapter and no abstraction until the second (ADR-0001), Rust for the core
  with Node only for the document toolchain (ADR-0002), polling before webhooks (ADR-0003),
  no rule language until three workflows exist (ADR-0004), single operator with actor
  identity first-class from the start (ADR-0005), and the roadmap as a live doc rather than a
  second tracker (ADR-0006).
- **The plan**, through five milestones, in [`docs/roadmap.md`](docs/roadmap.md).
- **The checks.** A pre-commit hook running the document invariants against the commit rather
  than the working tree, with CI as its backstop. A Cargo workspace whose lint configuration
  denies the escape hatches the quality bar prohibits. CodeQL over Rust, JavaScript and the
  workflows. A dependency audit and licence policy.
