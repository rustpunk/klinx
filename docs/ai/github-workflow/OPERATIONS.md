# GitHub Workflow Operations

## Purpose

Klinx uses a user-level GitHub Project, `Klinx Agent Delivery Board`, for conservative agent issue routing. These workflows provide lightweight maintenance checks; they do not replace human readiness review.

## Label Automation

Closed issues remove live agent-routing labels such as `agent-ready`, `needs-context`, `needs-grounding`, `needs-decision`, `needs-splitting`, `blocked`, `not-agent-ready`, and `agent-mode:*`.

Reopened issues remove stale ready/mode labels and, when the labels exist, add `not-agent-ready`, `needs-context`, and `needs-grounding`. Reopened issues must go back through grounding before they can re-enter the Agent Ready queue.

## Operational Workflows

- Agent Queue Audit reports `agent-ready` issues that still show conflicting labels, missing acceptance/verification text, or open `Blocked by:` refs.
- Agent PR Merged Closeout Audit checks that merged PRs close their linked issue, that live routing labels are gone, and that Project status is `Done` when Project data is visible to the workflow token.
- Agent Label Sync is manual and dry-run by default; it creates or updates the Clinker-style workflow label set plus Klinx's local helper labels.
- Agent Stale Status Reminders comments on stale `Agent Running` / `PR Open` Project items after a cooldown. It does not change labels or Project fields.

## Agent Snapshot And Update Helper

Use `scripts/gh-agent-snapshot.sh` before raw `gh api` calls when running GitHub agent skills. The wrapper defaults to repo `rustpunk/klinx`, user Project owner `rustpunk`, and Project number `3`; it delegates to the shared helper at `/home/glitch/.agents/skills/_shared/scripts/gh-agent-snapshot.sh`.

Read commands return compact JSON with `meta`, `items`, and precomputed `findings`. Issue, queue, Project status, and closeout snapshots include visible ProjectV2 fields in both `projectItems[].fields` and typed `projectItems[].fieldValues[]` form so agents do not need separate GraphQL reads for Project metadata:

- `scripts/gh-agent-snapshot.sh queue --milestone <name-or-number>`
- `scripts/gh-agent-snapshot.sh issues --issues <number,number,number>`
- `scripts/gh-agent-snapshot.sh issues --file issues.json`
- `scripts/gh-agent-snapshot.sh issue --issue <number>`
- `scripts/gh-agent-snapshot.sh project --status "Agent Ready"`
- `scripts/gh-agent-snapshot.sh closeout --pr <number>`

Use `issues` for decision gates, readiness batches, handoffs, or any workflow that inspects more than one issue. Do not loop `issue --issue` when the issue numbers are known upfront.

Use `queue --milestone` for milestone queues. It discovers issue numbers first and hydrates them through bounded bulk issue snapshots to avoid GitHub GraphQL node-limit failures.

Compact mode truncates emitted comment bodies, but helper readiness findings scan fetched comment text. Use `--full-comments` when the output needs complete comment bodies.

Bulk updates are dry-run by default and should be prepared as structured JSON:

```json
{
  "updates": [
    {
      "issue": 123,
      "addLabels": ["agent-ready"],
      "removeLabels": ["not-agent-ready", "needs-grounding"],
      "projectFields": {
        "Status": "Agent Ready",
        "Risk": "Low",
        "Verification": "required"
      }
    }
  ]
}
```

Run `scripts/gh-agent-snapshot.sh update --file updates.json` to inspect the planned operations. Add `--apply` only when the workflow intentionally mutates labels or Project fields. Project field updates support ProjectV2 single-select, text, date, and number fields and preflight the whole batch before applying labels or fields.

## Token Notes

Project-aware workflows use `secrets.PROJECT_TOKEN || github.token`. A repository token can read issue and PR data, but a user-level Project may require `PROJECT_TOKEN` with Project access for full status visibility.
