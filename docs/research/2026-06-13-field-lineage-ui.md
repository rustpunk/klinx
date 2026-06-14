# Field/column-level lineage UI — prior art, what works, failure modes

Deep-research report backing epic #64 (field-level lineage + explicit field ports on the
canvas). Method: 5-angle fan-out web search → 20 sources → 98 candidate claims →
3-vote adversarial verification (24/25 confirmed, 1 killed). Generated 2026-06-13.

## Bottom line

Successful column-level lineage tools converge on one rule: **never draw all field edges at
once.** DataHub, dbt, and Google Dataplex all default to table/node-level lineage and reveal
per-column edges only on demand. Rendering every column of every node at full detail is, in a
vendor's own words, "unreadable." The remedy is **overview-first, details-on-demand**
disclosure scoped to a single selected field.

## Verified findings (each 3-0 unless noted)

1. **Cardinal failure mode — all-edges-at-once is unreadable.** DataHub: "A graph that renders
   every column in every table as a separate node at full detail is unreadable." EuroVis
   "Unfolding Edges" (Bludau, Dörk & Tominski, CGF 42(3) 2023): on-edge encoding "applied to all
   edges… does not scale well… clutter and overcrowded displays."

2. **Remedy — progressive disclosure (one field on demand).** Convergent default across DataHub
   (column detail off by default; toggle/expand/hover), dbt (expand one column → its lineage
   graph, not all columns), and the EuroVis mantra (overview → select an edge → unfold in situ).

3. **Focus+context controls.** Dataplex documents *highlight* mode (emphasize matches, keep the
   full graph) vs *filter* mode (hide non-matches, keep connecting paths, collapse intermediates),
   plus degree/hop limits and incremental expansion ("five nodes at a time"; 3 of 10 levels
   expanded by default).

4. **Per-field ports are the universal primitive.** React Flow handles, Reaflow `Port`, Rete
   `Socket`, litegraph slots — a node hosts an arbitrary list of individually-addressable field
   ports and an edge binds to a specific source/target port (`sourceHandle`/`targetHandle`,
   `fromPort`/`toPort`). Field-to-field edge addressing is established prior art.

5. **Pass-through / identity clutter — label, don't hide.** Dataplex edge labels ("Exact copy"
   vs "Other"); dbt's "column evolution" lens distinguishes transformed vs reused/passthrough/
   rename, color-coded. Provide a "collapse pass-through" toggle rather than silently dropping
   edges.

6. **⚠️ Edge bundling is contraindicated for lineage.** Wu et al. (Entropy 2018): bundling is
   "inherently not information faithful" (many distinct networks → one bundled drawing) and can
   show an "illusion edge" between unconnected vertices. Since exact source→consumer mapping is
   the point of lineage, bundling defeats it. Useful only for coarse structure.

7. **Layout — ELK-style layered (Sugiyama).** Five phases (cycle-break, layer, crossing-min,
   placement, routing). Crossing minimization orders **nodes and ports** (barycenter/median);
   routing supports POLYLINE / **ORTHOGONAL** / SPLINES. Reaflow and React Flow delegate to
   elkjs. klinx already has a barycenter layered layout (`pipeline_view::layout_positions`) to
   extend — no JS/elkjs (klinx is Dioxus/SVG/Rust desktop).

## Recommendations for klinx

1. Default to node-level lineage; render per-field ports and reveal **one** field's upstream+
   downstream lineage on hover/select (progressive disclosure).
2. Highlight mode (dim non-lineage, keep graph) + filter mode (collapse off-path) for focus+context.
3. Cap disclosure by hop/degree; expand incrementally on large graphs.
4. Model fields as individually-addressable ports; bind edges to specific source/target ports.
5. Encode transformation type via edge label/color (exact-copy vs transform vs rename/passthrough)
   + a "collapse pass-through" toggle — don't hide identity edges.
6. ELK-style layered layout with port-aware crossing minimization + **orthogonal** routing;
   **avoid edge bundling**.

Phasing maps to: P1 #66 (1, partial 4 — hover-reveal + per-field ports), P2 #67 (2, 3, 5, 6),
P3 #68 (engine-resolved accuracy + wide-schema virtualization).

## Caveats

Production-tool evidence concentrates on DataHub / dbt / Dataplex; OpenLineage/Marquez, Atlan,
Collibra, Manta, SQLLineage, Spline produced no surviving claims. ETL canvases (NiFi, Airbyte,
Dagster, Prefect, KNIME, Alteryx, Talend) and creative node editors (Blender/Houdini/n8n) yielded
no surviving field-level claims — they may render node-level only (unconfirmed; open question).
The EuroVis/bundling papers concern general multivariate graphs, not lineage specifically — a
sound but reviewer-supplied analogy. Library port primitives are JS/DOM; klinx must render ports
as SVG and port/extend a layered layout in Rust. dbt/DataHub column features are paid-tier
(availability note, not a behavior contradiction).

## Sources

- https://datahub.com/blog/column-level-lineage-comes-to-datahub/
- https://docs.getdbt.com/docs/explore/column-level-lineage
- https://cloud.google.com/dataplex/docs/lineage-views
- https://onlinelibrary.wiley.com/doi/full/10.1111/cgf.14831 (EuroVis "Unfolding Edges", CGF 2023)
- https://www.ncbi.nlm.nih.gov/pmc/articles/PMC7513140/ (Wu et al., Entropy 2018, edge-bundling fidelity)
- https://eclipse.dev/elk/blog/posts/2025/25-08-21-layered.html
- https://eclipse.dev/elk/reference/options/org-eclipse-elk-edgeRouting.html
- https://reactflow.dev/learn/customization/handles · https://github.com/reaviz/reaflow
- https://rete.readthedocs.io/en/latest/Sockets/ · https://github.com/jagenjo/litegraph.js/
