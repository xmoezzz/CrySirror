# ADR 0105 — Self-admitted technical debt tracks unfinished work

Accepted 2026-11-19. Retired 2026-11-26.

## Context

Experiments and staged implementations can leave missing branches, temporary
behavior or unresolved compatibility contracts. A code move can make those
gaps harder to find if the original note no longer points to the current code.

## Decision

Record every incomplete experiment and implementation as self-admitted
technical debt (SATD) when it is created or discovered. Use a `SATD.md` next
to the owning component and a stable ID such as
`SATD-component-operation`. Reference the ID at the affected code or
contract. The record names the current location, affected behavior and
versions, work already in place, remaining scope and evidence required for
closure. A record may cover several cases that share one completion criterion.

Preserve the record while any required behavior is unresolved.

| Change | Required record update |
| --- | --- |
| Implement some cases | Append the date, completed cases and remaining scope. Keep the original ID and history. |
| Move or rename code | Move its references, update the current location, and append the date, old path, new path and effect on the remaining work. |
| Transfer component ownership | Move the canonical entry and keep its ID and complete history. Update incoming references. |
| Withdraw an unfinished experiment | Keep the record and append the date, reason and last code location. |
| Complete the tracked scope | Verify all required cases through their normal entry points. Record the evidence in the completing change or the owning component's validation notes, then delete the entry and its references in that change. |

Keep draft native syscalls isolated until their full ABI contract is
implemented. SATD tracks incomplete drafts and existing defects. The
activation and acceptance rules remain in the
[engine semantics skill](../../.agents/skills/crysirror-engine-semantics/SKILL.md).
