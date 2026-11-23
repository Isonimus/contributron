# Contributing to Contributron

Thanks for your interest. These notes make a contribution land smoothly.

Be aware of what stage this is at: **nothing is built yet.** The repository currently holds
its decisions, its plan and the checks that enforce both. If you want to help, the most
useful contributions right now are arguments against a recorded decision, not code.

## Setting up

You need Rust (the exact toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml)
and `rustup` installs it on the first `cargo` command) and Node.js 22 for the document
toolchain.

```bash
cargo fmt --all --check                               # formatting
cargo clippy --all-targets --all-features -- -D warnings   # the lint gate
cargo test --workspace                                # the test suite
npm run lint                                          # the document invariants
```

To install the hook that checks the decision records on every commit:

```bash
ln -sf ../../.claude/hooks/pre-commit .git/hooks/pre-commit
```

Node carries no application code. It is here to run the vendored
[stele](https://www.npmjs.com/package/@isonimus/stele) toolchain and nothing else
(ADR-0002); `package.json` has no dependencies by design.

## What a change needs

The engineering standards are in [`docs/quality-bar.md`](docs/quality-bar.md), and the rules
specific to this project are the standing invariants in [`CLAUDE.md`](CLAUDE.md), section 4.
They are written for AI-assisted work and they apply to everyone. In short:

- **A feature is a slice, and the slice comes first.** Write `slices/<name>.md` with a
  `## Definition of Done` as Given/When/Then scenarios *before* the code, and a
  `## Verification` section naming the proof for each scenario. The linter requires both.
  At merge, the slice is frozen into past tense: this is what shipped.
- **A decision later work must obey is an ADR**, not a paragraph in a pull request. Changing
  a decision means a new ADR that supersedes the old one and says why the old reasoning was
  wrong. Never edit a committed one.
- **Logic has a regression test that bites.** It fails before the fix and passes after; one
  that passes before proves nothing. Test observable behaviour, not internals, and cover the
  error paths. Tests are codebase and are held to the same standards.
- **No escape hatch that turns off the checker.** No `unsafe`, no `unwrap` or `expect`
  outside tests and process startup, no `#[allow(...)]` without the reason beside it. Clippy
  enforces most of this; where it cannot, review does.
- **Nothing is normalized at the wrong boundary.** Provider field names stop at the adapter.
  A domain type that mentions a GitHub concept is a defect (ADR-0001).
- **Open work goes in the ledger.** [`LEDGER.md`](LEDGER.md) is the only tracker. Closing an
  item means deleting its line. GitHub issues mirror it for navigation; where the two
  disagree, the ledger wins (ADR-0006).

CI runs formatting, clippy as a gate, the test suite, a locked build, the document
invariants, CodeQL over Rust, JavaScript and the workflows themselves, and a dependency audit
on every pull request. A pull request needs it green.

## Changes that touch authority

Some changes are more expensive to get wrong than others. If a contribution affects what an
agent is permitted to do, what the policy engine enforces, which credentials reach which
process, or what an operator is asked to approve, say so explicitly in the pull request and
expect the review to be slower.

The two principles those changes are measured against:

- **The engine is the security boundary, never the agent.** Nothing in the enforcement path
  reads what an agent says about what it should be allowed to do.
- **The provider is the final authority on provider state.** Mergeability, approvals and
  branch protection are re-fetched immediately before any sensitive operation, whatever the
  event stream claimed.

A change that makes an agent more autonomous without making the enforcement around it
stronger will be turned down, even if it works.

## Commits and pull requests

Conventional commit messages. Explain **why**, not what: the diff already shows what. One
logical change per commit.

Pull requests use the template. The checklist is not ceremony: each line corresponds to
something the linter, the hook or CI will otherwise catch later and more annoyingly.

## About `adr/` and `slices/`

If the layout looks unusual, that is deliberate. Every durable decision is an immutable ADR
and every feature a slice, both written before the code and shipped in the same commit, so
the record of *why* is never rewritten, only superseded. The commit history is meant to read
as a trail of decisions. [`adr/INDEX.md`](adr/INDEX.md) is generated; do not edit it by hand.

`slices/` does not exist yet. It appears with the first feature.

## Reporting bugs and asking for features

Use the issue templates. For anything where an agent did something it should not have been
able to do, use the Security tab instead: [`SECURITY.md`](SECURITY.md) explains what counts.

Before filing a feature request, read [`docs/roadmap.md`](docs/roadmap.md) and the open
issues. A good deal is deliberately deferred rather than overlooked, and each of those
issues carries its reason. Arguing against the reason is more useful than a fresh request.
