# Aggregate Field-Lineage UI Notes

Generated for issue #120.

## Recommendation

Aggregate nodes should show a grouped output record, not a full input schema.
Render group-by keys first, followed by aggregate emit targets. Field-lineage
edges should remain demand-revealed by hover/click/pin rather than rendered
globally.

For a case like `source_b.field` used only by `group_by: [source_b.field]`, the
Aggregate node should expose a `source_b.field` group-key row. Its field-lineage
edge should run from the producer's `field` row to that Aggregate group-key row.
This makes the output grouping key inspectable without implying that all other
input fields pass through the aggregate.

## Rationale

- Column-level lineage tools converge on progressive disclosure: keep the
  graph readable first, then reveal exact field dependencies on demand.
- Group keys are part of the aggregate output identity. Showing them as
  first-class rows answers "what defines this grouped record?" and gives the
  lineage edge a stable endpoint.
- Aggregate emit targets are separate output values derived from aggregate
  expressions; they should not be mixed with unrelated carried input columns.
- A future UI pass can visually label or group the first rows as "Group keys",
  but the model should not require that UI before exposing correct lineage.

## Sources

- DataHub column-level lineage: https://datahub.com/blog/column-level-lineage-comes-to-datahub/
- dbt column-level lineage: https://docs.getdbt.com/docs/explore/column-level-lineage
- Google Dataplex lineage views: https://cloud.google.com/dataplex/docs/lineage-views
- Shneiderman information-seeking mantra: https://www.cs.umd.edu/~ben/papers/Shneiderman1996eyes.pdf
- ELK layered layout phases: https://eclipse.dev/elk/blog/posts/2025/25-08-21-layered.html
