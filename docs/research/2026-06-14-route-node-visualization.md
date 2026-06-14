# Route / Switch node visualization in a DAG canvas — best practices & failure modes

Deep-research report backing the klinx Route-node canvas work (example migration #74; Route
field-lineage in epic #64 / Phase-2 #67; the route-filtered-lineage idea). Method: 6-angle
fan-out web search → 22 sources → 99 candidate claims → 3-vote adversarial verification
(**25/25 confirmed, 0 killed**, 11 findings after synthesis). Generated 2026-06-14.

Context: klinx's engine has a dedicated `route` node — `RouteBody { mode: Exclusive|Inclusive,
conditions: IndexMap<branch, cxl_predicate>, default }`; downstream nodes consume a specific
branch via `route_name.branch`. A route passes all input columns unchanged to whichever branch a
record matches. Klinx is a Dioxus/SVG desktop canvas (no React Flow / elkjs — patterns are
*structure to reimplement*, not libraries to drop in).

## Bottom line

Established node-graph tools converge hard: **render each named branch as its own output port on a
single node card, label the branch AT THE PORT (never on the edge), bind each downstream edge to a
specific branch port, and model the required default/fallback as a first-class, always-present,
visually-distinct port — not just "another rule."** Show the predicate expression on demand (inspector/
tooltip on selection), keep only the branch *name* on the port. For "select a branch and dim the
non-matching paths," the precise prior-art term is **highlight / focus+context** (emphasize the
matching path, keep the full graph), distinct from **filter** (hide/collapse non-matching) — Dataplex
ships both as explicit modes. Do **not** bundle per-branch edges; bundling measurably hurts
single-path tracing.

## Verified findings (confidence; vote)

1. **One output port per branch; bind edges to a specific branch.** [high, 3-0] Unanimous across
   n8n (one rule → one output), Node-RED (output count derived from rule count), Unreal Blueprint
   (each case = a separate exec pin), AdvancedControlFlow (`AddCasePinPair()` per branch), and
   React Flow (multiple source handles with unique ids; edges bound via
   `sourceHandle`/`targetHandle`). The one-output-per-branch topology with edges bound to a
   specific port is the primary, cross-tool pattern.

2. **Label the branch at the PORT, not the edge.** [high, 3-0] Every surveyed tool with named
   branches labels the *port/pin*; the wire carries no native label. Port labels stay legible at
   scale (the label persists even when edges are dense or hidden). n8n names the output; Unreal
   labels the pin by case name; AdvancedControlFlow sets `PinFriendlyName` and re-numbers on
   insert/remove. No surveyed tool labels the branch on the wire.

3. **Default/fallback = a first-class, dedicated, always-present, distinct port.** [high, 3-0 —
   the strongest convergence in the survey; 5 independent tools] n8n "Fallback Output"
   (None / Extra Output / Output 0); Node-RED "Otherwise" rule kind; NiFi `RouteOnAttribute`
   hard-coded `unmatched` relationship (always present, must be wired or auto-terminated); Unreal
   "Default" pin; AdvancedControlFlow reserved `DefaultExec` pin at a fixed index. For klinx's
   **required** default, the strongest precedent is NiFi / AdvancedControlFlow: an always-present,
   reserved, distinctly-styled/positioned port (e.g. last/bottom), not merely another condition row.

4. **Predicate text → details-on-demand, not inline.** [high, 3-0 on the principle; 2-1 on the
   specific application] Shneiderman's mantra ("overview first, zoom & filter, then
   details-on-demand"; details = "click an item for a pop-up of its attributes") and Airflow's
   real DAG canvas (state shown "by color and tooltip"; "hover for more detail" rather than
   persistent on-node text) both prescribe: keep the concise branch **name** on the port, put the
   full cxl predicate in the inspector / hover. (AdvancedControlFlow reinforces this — its
   predicate enters via a separate input wire, never as text on the output port.)

5. **Two distinct branch/lineage focusing modes exist as prior art: highlight vs filter.** [high,
   3-0] Google Cloud Dataplex, verbatim: **Highlight** — "matching nodes are visually emphasized
   with colors and borders, while the full graph remains visible"; **Filter** — "non-matching
   nodes are hidden, and the graph is simplified to show only matching nodes and the paths between
   them" (off-path assets collapse into grouped nodes). Plus a **"Visualize path"** action:
   selecting a node highlights only its direct lineage path. **Terminology caveat:** the
   "dim/reduce-opacity non-matching" idea is *highlight / focus+context*, NOT Shneiderman-filter
   (Dataplex emphasizes via colour+borders or hides — it never literally dims). Decide deliberately
   which klinx implements; they are different interactions.

6. **Lay the selected branch's lineage near the geodesic (straight) line to its target; push
   non-matching branches away.** [high, 3-0; single peer-reviewed source, small n=16] Eye-tracking
   (Huang 2013): 94% of path-task subjects first search the path nearest the straight line to the
   target and miss correct-but-distant paths; gaze slips into target-pointing branch edges even
   when off-path. Design implication (the paper's own): route important paths close to the
   geodesic, irrelevant branches further away. Applied: on branch focus, route the emphasized
   lineage near the straight line and displace/de-emphasize the rest.

7. **Edge bundling is an anti-pattern for per-branch tracing.** [high, 3-0; single controlled
   study] McGee & Dingliana (AVI 2012): bundling "negatively impacts user performance at tracing
   paths between nodes, both in accuracy and time"; it helps only coarse cluster-overview tasks.
   A route user must follow one branch to a specific downstream node → never bundle branch edges.
   (Consistent with epic #64's existing "avoid edge bundling" rule.)

8. **Prefer large (near-90°) crossing angles over exhaustive crossing elimination.** [high, 3-0;
   single source — the canonical RAC result] Huang 2013: large-angle crossings keep gaze smooth;
   acute crossings cause slow back-and-forth gaze. A few more crossings at large angles ≈ same
   readability. For a node fanning out multiple ports, optimise crossing *angle*, not just count.

9. **Port-explosion UX: inline add/remove pin management with auto-re-indexing.** [high, 3-0;
   single third-party source] AdvancedControlFlow: an `[Add Pin]` affordance + port-targeted
   context menu ("add case before/after", "remove this/first/last") that re-writes subsequent pin
   names/labels so they stay consistent. Concrete pattern for many-branch nodes (klinx authoring,
   later).

10. **High-degree fan-out: downsample relationships rather than draw every edge.** [medium, 3-0;
    weakest source — 2021 vendor blog] DataHub downsamples for nodes with hundreds of entities.
    Most route nodes have few branches, so this matters for *downstream fan-out*, not the route's
    own branch count.

11. **Organizing frame = Shneiderman's mantra.** [high, 3-0] Full DAG = overview; select a branch +
    dim/hide non-matching = zoom/filter; predicate/branch attributes in the inspector =
    details-on-demand. Treat as the default frame, not a hard constraint (the known critique:
    overview-first favours novices over power-users).

## Recommendations for klinx (mapped to the engine shape)

1. **Render the Route card with one output anchor per branch**, stacked top-to-bottom in
   `conditions` declaration order (`IndexMap` preserves it), each row labelled with the branch
   **name** only. This mirrors the field-row anchor geometry already built for field lineage —
   reuse the per-row anchor machinery rather than the single node-level `port_out`.
2. **Give the `default` branch a reserved, visually-distinct port** (distinct style + fixed
   position, e.g. last/bottom), always present — per the 5-tool convergence (finding 3).
3. **Bind each downstream edge to its branch port.** klinx's YAML already carries the branch
   (`NodeInput::Port { node, port }`); resolve the consumer's edge to the specific branch anchor
   instead of the node-level port (today `node_input_name` discards the port — that's the gap to
   close).
4. **Predicate in the inspector / on hover, not on the card** (finding 4). Optionally a short
   predicate inline with truncation + tooltip fallback (open question — no comparative evidence).
5. **Field lineage through a Route = pure passthrough fan-out:** each branch carries the input
   column set unchanged. Per-branch field edges are identity carries (Passthrough kind) to each
   branch port — no derives. (Matches the engine: Route has no `cxl`.)
6. **Route-filtered lineage (the user's idea) = highlight / focus+context, not filter.** Selecting
   a branch emphasises its downstream lineage and **dims** (reduces opacity of) the non-matching
   branches' paths, keeping the full graph visible (finding 5; the dim is focus+context). Route the
   emphasised path near the geodesic line, displace the rest (finding 6). Reuse the existing
   `.klinx-node--dimmed` / `--recede` dimming already used for field-hover.
7. **Never bundle branch edges** (finding 7); optimise crossing **angle** (finding 8); for many
   branches add inline pin management later (finding 9).

## Caveats

Findings 1–5, 11 rest on multiple primary sources (vendor docs + first-party source + the
Shneiderman paper), unanimous — safe to build on. Findings 6, 7, 8 each rest on a single
peer-reviewed study (Huang 2013 geodesic/RAC, small n=16, partly exploratory; McGee & Dingliana
2012 bundling) — strong but not independently replicated here. Finding 9 is a single third-party UE
plugin (a concrete pattern, not an industry standard). Finding 10 is the weakest (2021 vendor blog
— medium confidence; verify currency). **Scope nuances:** NiFi/Unreal are prior art for *topology
and labeling*, not drop-in (klinx is SVG/Dioxus); React Flow's `sourceHandle`/`targetHandle` model
is *structure to reimplement*. **Behavioural nuance:** n8n and Node-RED default to evaluating ALL
rules (multi-match fan-out) unless "stop after first match"; klinx's `Exclusive` mode is
first-match — do not inherit the multi-match default, but the topology lessons hold for both
`Exclusive` and `Inclusive`.

## Open questions

- Branch-focus default: **highlight** (keep full graph, emphasise) vs **filter** (hide/collapse)
  vs offer both (Dataplex). No evidence on which suits a pipeline-*authoring* IDE vs a lineage-
  *exploration* tool. (Recommend highlight/dim first — it's what the user described.)
- How should branch colour propagate to downstream edges/nodes? No surveyed tool documents
  propagating a branch colour along its downstream lineage — needs dedicated research (React Flow
  edge styling, Sankey-style coloured flows).
- Legibility threshold (#branches) before port-side labels need collapsing/scrolling/overflow.
- Predicate display: hover-tooltip vs docked inspector for a desktop SVG IDE; inline-truncation
  threshold for short predicates.

## Sources (primary unless noted)

- n8n Switch — https://docs.n8n.io/integrations/builtin/core-nodes/n8n-nodes-base.switch/
- Node-RED Switch — https://flowfuse.com/node-red/core-nodes/switch/ · https://nodered.org/docs/user-guide/nodes
- NiFi RouteOnAttribute — https://docs.cloudera.com/cdf-datahub/7.3.1/nifi-components-cfm4/docs/nifi-docs/components/org.apache.nifi/nifi-standard-nar/x/org.apache.nifi.processors.standard.RouteOnAttribute/index.html
- Airflow UI — https://airflow.apache.org/docs/apache-airflow/stable/ui.html
- Unreal Blueprint flow control — https://docs.unrealengine.com/4.26/en-US/ProgrammingAndScripting/Blueprints/UserGuide/FlowControl
- AdvancedControlFlow (UE plugin) — https://github.com/cdpred/UEPlugin-AdvancedControlFlow
- React Flow handles — https://reactflow.dev/learn/customization/handles
- Shneiderman, "The Eyes Have It" (1996) — https://www.cs.umd.edu/~ben/papers/Shneiderman1996eyes.pdf
- Huang, eye-tracking graph readability (geodesic / RAC) — https://www.ieeesmc.org/wp-content/uploads/2015/09/tc-vac-paper.pdf
- McGee & Dingliana, edge bundling & path tracing (AVI 2012) — https://dl.acm.org/doi/pdf/10.1145/2254556.2254670
- Google Cloud Dataplex lineage views (highlight/filter/visualize-path) — https://docs.cloud.google.com/dataplex/docs/lineage-views
- DataHub lineage downsampling (blog, 2021) — https://datahub.com/blog/data-in-context-lineage-explorer-in-datahub/
- Carbon data-vis colour palettes — https://carbondesignsystem.com/data-visualization/color-palettes/
