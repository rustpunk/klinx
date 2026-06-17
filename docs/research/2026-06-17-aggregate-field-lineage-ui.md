# Aggregate Field-Lineage UI Notes

Generated for issue #120.

## Recommendation

Aggregate nodes should show both the grouped-output record and the semantic
operator inputs that define it. Render `group_by` keys as a compact input role
section above the output fields, then render the grouped output record as normal
field rows: de-duplicated group keys first, followed by aggregate emit targets.
Field-lineage edges should remain demand-revealed by hover/click/pin rather
than rendered globally.

For a case like `source_b.field` used only by `group_by: [source_b.field]`, the
Aggregate node should expose a `group_by source_b.field` input role row. Its
role-lineage edge should run from the producer's `field` row to that
`group_by` row. If the key is also part of the grouped output record, the normal
field-lineage edge should run to the Aggregate's output field row separately.
This makes the grouping dependency inspectable without implying that all other
input fields pass through the aggregate or drawing two cables into the same row.

## Rationale

- Column-level lineage tools converge on progressive disclosure: keep the
  graph readable first, then reveal exact field dependencies on demand.
- Group keys are operator inputs and part of the aggregate output identity.
  Showing them in a role section answers "what defines this grouped record?"
  while keeping output rows reserved for fields that actually leave the node.
- Aggregate emit targets are separate output values derived from aggregate
  expressions; they should not be mixed with unrelated carried input columns.
- A role-port row mirrors the Route/Cull branch-port precedent: semantic ports
  get their own row and anchor instead of overloading generic node-level labels.

## Sources

- DataHub column-level lineage: https://datahub.com/blog/column-level-lineage-comes-to-datahub/
- dbt column-level lineage: https://docs.getdbt.com/docs/explore/column-level-lineage
- Google Dataplex lineage views: https://cloud.google.com/dataplex/docs/lineage-views
- Shneiderman information-seeking mantra: https://www.cs.umd.edu/~ben/papers/Shneiderman1996eyes.pdf
- ELK layered layout phases: https://eclipse.dev/elk/blog/posts/2025/25-08-21-layered.html
