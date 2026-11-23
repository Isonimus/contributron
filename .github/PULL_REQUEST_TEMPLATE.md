## Summary

<!-- What this changes and why. The reasoning, not a restatement of the diff. -->

Closes #

## Checks

- [ ] `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings` and
      `cargo test --workspace` all pass
- [ ] `npm run lint` passes, and the pre-commit hook ran (not bypassed with `--no-verify`)
- [ ] A slice in `slices/` carries the Definition of Done, written before the code, and its
      `## Verification` names the proof for each scenario
- [ ] A decision later work must obey is an ADR in `adr/`, not a paragraph in this
      description
- [ ] `LEDGER.md` updated: a new defect or deferral added, a closed item's line deleted
- [ ] `README.md`, `docs/roadmap.md` and `CHANGELOG.md` updated if this changes what a
      person sees, does, or can expect next

## Notes for review

<!-- Anything a reviewer should look at first, or a choice that looks wrong but is
     deliberate. If this changes agent permissions, provider authority or what an operator
     is asked to approve, say so here: those are the parts where a mistake is expensive. -->
