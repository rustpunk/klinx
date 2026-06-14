# Field-lineage UI follow-ups — implementation research

Companion to [`2026-06-13-field-lineage-ui.md`](2026-06-13-field-lineage-ui.md) (prior-art deep
research) and the session handoff `notes/handoffs/2026-06-14-field-lineage-ui-followups.md`.
Scope: ground the two confirmed field-lineage UI enhancements that extend epic **#64** with exact
code-level findings so they can be filed as sub-issues (candidates alongside Phase-2 **#67**) and
implemented later. **Research only — nothing here is implemented.** Generated 2026-06-14.

Engine facts below are cited against the git-pinned clinker rev `c233a38` in the Cargo checkout
(`~/.cargo/git/checkouts/clinker-*/c233a38/…`); klinx-side facts against the working tree.

---

## Issue 1 — pointer-driven hover/click model with three relationship types

### 1.1 What exists today

The headless lineage model is a flat edge list with a two-way classification:

- `FieldEdge { from_node, from_field, to_node, to_field, passthrough: bool }`
  (`pipeline_view/field_lineage.rs:44-51`). `passthrough: true` = identity carry (`c → c`,
  unchanged value); `passthrough: false` = derive (an input column feeds an `emit`-produced
  output field).
- `compute_field_lineage` emits, per transform node: **derive** edges from each emit's
  let-resolved support (`pipeline_view.rs:834-855`) and **identity** edges for every input column
  not shadowed by an emit (`pipeline_view.rs:859-871`).
- The reveal is driven by **one** signal, `hovered_field: Signal<Option<(usize, String)>>`
  (`components/canvas/panel.rs:155`). Only a **field-row** `onmouseenter` sets it
  (`components/canvas/node.rs:240-243`); node-whitespace hover and field **click** do nothing.
- On hover the panel computes the **direct 1-hop** neighbourhood via
  `lineage_closure` (every edge incident to the anchor, both kinds — `field_lineage.rs:258-268`),
  draws those `FieldConnector`s at full opacity, recedes the node-level cable `<g>`
  (`panel.rs:524-525`, `.klinx-canvas-edges--recede` opacity 0.12), and dims non-participating
  cards (`panel.rs:555`, `.klinx-node--dimmed`).
- Cables read as two colours: derive = verdigris solid, passthrough = text-floor muted
  (`connector.rs:64-69`; CSS `klinx.css:4304-4311`).

### 1.2 Target interaction model (pointer scope → reveal)

| Pointer location | Reveal |
| --- | --- |
| Not over any node | No field connectors (today: same). |
| Over a node, not on a row | **All** of that node's identity edges (`c → c`), colour-split passthrough vs used/access. **Not** derives. (today: nothing) |
| Over a specific row | That field's 1-hop incident edges (today's behaviour, kept). |
| **Click** a field | Escalate to a **more detailed** provenance view, pinned. (today: nothing) |

**Key correction (from the handoff):** on **node** hover the carries target the **same-named**
column on the consumer (`line_total → line_total`), never the emitted field
(`line_total → value_tier`). The derive (`→ value_tier`) belongs to the field **click** detail,
not the node overview. Today node hover doesn't exist, so there is no regression — but the
node-hover edge set must be the *carry* subset, not the full incident set.

### 1.3 The three relationship types — definitions and how to compute each

All three are computed **headlessly** in `compute_field_lineage`; the renderer only styles them.
Using the `value_tier` node example (predecessor produces `shipping_method, status,
order_date_parsed, line_total, …`; the node runs `emit value_tier = …line_total…`):

1. **Passthrough** — input column carried unchanged AND read by **no** emit.
   E.g. `shipping_method → shipping_method`. Today's `passthrough: true` edges minus type 2.
2. **Used / access** — input column carried unchanged (`c → c`) **but also** present in some
   emit's support set. E.g. `line_total → line_total`, because `value_tier`'s expression reads
   `line_total`. A subset of today's `passthrough: true` edges.
3. **Derive** — input column → the **emitted** output field. E.g.
   `orders.line_total → value_tier.value_tier`. Today's `passthrough: false` edges.

**Classifier for used/access (the only new logic):**
- `emit_supports(program)` already returns each emit's let-resolved input-column support
  (`field_lineage.rs:223-238`). Union them into `used_cols` (all columns any emit reads).
- When `compute_field_lineage` emits an identity edge for a carried column `c`
  (`pipeline_view.rs:859-871`): if `c ∈ used_cols` → **Access**, else → **Passthrough**.
- A column shadowed by an emit of the same name (`emit a = a + 1`) is **Emitted**, not a carry —
  it never reaches the identity-edge branch, so "used/access" correctly applies only to
  *non-shadowed* columns that feed *other* emits. (`a`'s self-reference is handled by the existing
  intra-node derive logic, `pipeline_view.rs:834-855`.)

**Recommended data-model change:** replace `FieldEdge.passthrough: bool` with
`kind: FieldEdgeKind { Passthrough, Access, Derive }` (a 3-way enum). This is a pure widening of
the existing 2-way split, keeps all classification headless and unit-testable, and the renderer
maps each variant to a stroke colour. The `value_tier` node's edge set becomes the canonical
fixture (one of each kind). `FieldConnector`'s `passthrough: bool` prop becomes the same enum.

### 1.4 Hover-state machine and event wiring

Replace the single `hovered_field` signal with a richer target plus a separate pin:

```text
HoverTarget = None | Node(usize) | Field(usize, String)
pinned:      Option<(usize, String)>      // set by click; survives pointer moves
```

The panel computes the active edge set from `pinned` if set, else from `HoverTarget`:
- `Node(n)` → that node's **carry** edges (Passthrough + Access incident to `n`), via a new
  headless helper `node_carry_edges(edges, n)`.
- `Field(n, f)` → today's `lineage_closure(edges, n, f)` (1-hop, both kinds).
- `pinned == Some((n, f))` → a new transitive helper (§1.6).

**Event wiring** leans on the fact that `mouseenter`/`mouseleave` **do not bubble** (unlike
`mouseover`/`mouseout`) — entering a child does not re-fire the parent's enter, and leaving to a
child does not fire the parent's leave. That gives a clean upgrade/downgrade ladder:

- `.klinx-node` card `onmouseenter` → `HoverTarget::Node(index)` (skip while `pinned`).
- `.klinx-node` card `onmouseleave` → if current target is on `index`, set `None`.
- field row `onmouseenter` → `HoverTarget::Field(index, name)` (upgrade Node→Field; the card's
  enter does not re-fire).
- `.klinx-node-fields` container `onmouseleave` → if current target is `Field` on `index`,
  **downgrade to `Node(index)`** (the pointer is still inside the card, just off the row list).
  Reuse today's index-guard pattern (`node.rs:158-168`) so a jump straight onto another node's
  row stays order-independent.
- field row `onclick` → toggle `pinned = Some((index, name))`; canvas-background `onclick`
  (already deselects, `panel.rs:501-504`) also clears `pinned`; clear `pinned` on the D1
  view-swap effect (`panel.rs:171-202`) and on collapse (`node.rs:131-135`), exactly as
  `hovered_field` is cleared today.

A field-less node yields an empty carry set → `Node(n)` reveals nothing (no dim), which is the
desired no-op.

### 1.5 Avoiding SVG re-render churn

Every hover writes a signal and re-renders the whole canvas — the existing **D2** caveat
(`panel.rs:152`), accepted for Phase-1's small graphs. Node-hover enlarges the active set (all
carries for one node) but it is still bounded by one node's column count, and the aggregation is a
cheap `O(edges)` filter. Recommendations:

- Keep the single-signal re-render for now; do **not** add per-row signal writes.
- The active-edge computation stays a pure function of `(HoverTarget|pinned, field_edges, stages)`
  in the panel body (as today, `panel.rs:210-223`) — no new effects.
- If churn becomes visible on wide nodes later, the documented Phase-2 perf path is to scope the
  highlight to affected cards/cables; out of scope here.

### 1.6 The click → "more detailed" detail view

"More detailed" than the 1-hop hover means **provenance**: trace an emitted column back to the
source columns that feed it. Recommended behaviour:

- Click an **Emitted** field → pin it and reveal its **transitive upstream** closure following
  Derive **and** Access/Passthrough edges to origin (Declared) rows, styled to mark every
  contributing input as feeding the `emit` computation. New headless helper
  `field_provenance(edges, n, f)` (transitive walk, distinct from the deliberately-1-hop
  `lineage_closure` — see its rationale at `field_lineage.rs:240-257`).
- Click a **Declared/PassThrough** field → pin its 1-hop both-direction view (so a click still
  "sticks" the hover for inspection without the pointer staying put).
- This is the only place a transitive walk is allowed; the prior research's cardinal rule is
  "never draw all field edges at once" and "one field on demand"
  (`2026-06-13-field-lineage-ui.md` findings 1–2). A pinned single-column provenance trace honours
  that (one field, on explicit demand) while node-hover stays carry-only.

### 1.7 Reconciling node-hover-shows-all with progressive disclosure

The prior research warns that all-edges-at-once is "unreadable" and recommends overview-first,
one-field-on-demand (`2026-06-13-field-lineage-ui.md` findings 1–3). Node-hover deliberately
shows *all carries of one node*, which is broader — weigh it explicitly:

- **Why it stays legible:** carries are near-horizontal 1:1 lines (`c → c`), not a fan; they don't
  cross. Excluding derives from node-hover is what keeps it an "overview" rather than the
  unreadable full graph. This matches Dataplex's *highlight* mode (emphasise a scoped set, keep
  the graph) over a full column-graph render.
- **Open design decision (recommend asking before implementing):** does `Node(n)` show only
  *incoming* carries (predecessor → `n`, "where do this node's columns come from") or *incoming +
  outgoing* (`n` → successors too)? Incoming-only is the tighter overview and matches the
  `line_total → line_total` framing in the spec; incoming+outgoing answers "carry path through
  this node" at the cost of more lines. **Recommendation: incoming-only** for the first cut,
  outgoing as a later toggle.
- **Wide-schema guard:** for very wide nodes even carries can clutter. The prior research's
  hop/degree caps and "collapse pass-through" toggle (findings 3, 5) are the documented escape
  hatch; defer to Phase 2/3, but file it as a known follow-up so node-hover isn't shipped as if it
  scales unbounded.

---

## Issue 2 — field datatype indicators

Goal: each field row shows its datatype inline, e.g. `line_total : float`, `status : string`,
legible in both themes. Source of truth is `cxl::typecheck::Type`.

### 2.1 Availability matrix (per `FieldKind`)

| FieldKind | Type source | Needs engine compile? | Difficulty |
| --- | --- | --- | --- |
| **Declared** (Source schema / input-port schema) | `ColumnDecl.ty` directly | No | Trivial |
| **PassThrough / Access** (carried column) | the producer row's type, propagated along the carry | No (if type is threaded onto `FieldRow`) | Easy |
| **Emitted** (`emit name = expr`) | typecheck the expression | Yes (or best-effort) | Harder |

### 2.2 Declared rows — directly available

`ColumnDecl` carries the type (`clinker-core/src/config/pipeline_node.rs:722-726`):

```rust
pub struct ColumnDecl {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: cxl::typecheck::Type,
}
```

Both lineage origins already read `ColumnDecl`: a pipeline Source via `body.schema.columns`
(`pipeline_view.rs:995`) and a composition input port via `decl.schema.columns`
(`pipeline_view.rs:930`), both through `declared_rows` (`pipeline_view.rs:706-714`). So Declared
rows can show types with **zero** new engine work — just carry `ty` through `declared_rows`.

`cxl::typecheck::Type` (`cxl/src/typecheck/types.rs`) is a small enum: `Null, Bool, Int, Float,
String, Date, DateTime, Array, Map, Numeric, Any, Nullable(Box<Type>)`. It has
`display_name() -> &'static str` and a `Display` impl. **Verify before relying on it:** whether
`Display`/`display_name` renders `Nullable(Int)` with the inner type or just `"Nullable"`. If it
drops the inner type, add a klinx-side compact formatter (`Int`, `Float`, `String`, `Bool`,
`Date`, `DateTime`, `Array`, `Map`, `Numeric`, `Any`, and `T?` for `Nullable(T)`) so the suffix
stays informative and short.

### 2.3 PassThrough / Access rows — propagate for free

A carried column's type equals the producer column's type. If `FieldRow` gains an
`Option<Type>` (or a pre-rendered short string), the carry branches in `compute_field_lineage`
already copy the column name from the producer's row — copy the type alongside, and Declared types
flow through every downstream carry with no compile. This covers types 1 and 2 of Issue 1 too.

### 2.4 Emitted rows — typecheck, two paths

Emitted-field types require typing the expression. Two options, in increasing fidelity:

1. **Engine-accurate via `CompiledPlan` (preferred, ties to #68).** When `state.compiled_plan` is
   present (Resolved view), `CompiledPlan::typed_output_row(name) -> Option<&Row>`
   (`clinker-core/src/plan/compiled.rs:84`) gives a node's fully-typed output row — types for
   **all** output columns (emitted + carried). No re-implementation of inference; this is the
   Phase-3 engine-resolved surface the epic already plans for. Unavailable in Raw view / for an
   unvalidated composition being edited live.
2. **Best-effort headless typecheck (Raw / composition path).** `cxl::typecheck::type_check(
   resolved: ResolvedProgram, schema: &Row) -> Result<TypedProgram, Vec<TypeDiagnostic>>`
   (`cxl/src/typecheck/pass.rs:133`). Build an input `Row` from predecessor column types, resolve
   the parsed program, typecheck, then read per-emit types exactly as clinker's
   `bind_schema::propagate_row` does (`clinker-core/src/plan/bind_schema.rs:2783`):
   `typed.types.get(expr.node_id()).cloned().unwrap_or(Type::Any)` over
   `for_each_field_emit`. More moving parts (needs the resolver step and a constructed `Row`); on
   diagnostics from half-typed live edits, fall back to no type / `Any` per the existing
   never-panic, degrade-gracefully rule (`field_lineage.rs:143-150` sets the precedent for parse
   errors).

### 2.5 Compact row layout

- Add a `.klinx-node-field-type` span after `.klinx-node-field-name`
  (`node.rs:244-246`): `flex: 0 0 auto`, muted colour, `text-overflow: ellipsis`, small left gap.
  Keep name `flex: 1 1 auto` ellipsis so a long name truncates before the type.
- Row stays exactly `FIELD_ROW_HEIGHT` (22px) with `overflow:hidden` — the geometry contract with
  `field_row_y` (`pipeline_view.rs:152-154`, `klinx.css:4221-4238`) is unchanged because the type
  span lives inside the existing flex row.
- Colours: reuse the muted neutral (`--klinx-text-floor` / `--klinx-iron`) for the type so it
  reads as secondary to the name. Add an Enamel override mirroring the existing field-name ones
  (`klinx.css:247-255`) so the dim type stays legible on the light plate.
- Types are short (`Int`/`Float`/`String`/…), so the suffix rarely truncates at 160px node width.

---

## Issue 3 — clinker's dedicated Route shape + example migration (filed as #74)

`examples/pipelines/order_fulfillment.yaml` fakes routing: a `type: transform` (`route_priority`)
emits a synthetic `_route` string, and two `output` nodes both read that same node. Clinker has a
dedicated `route` node — `PipelineNode::Route { header, config: RouteBody }` where
`RouteBody { mode: Exclusive|Inclusive, conditions: IndexMap<branch, CxlSource>, default }`
(`clinker-core/src/config/pipeline_node.rs:94-98, 1068-1079`). Downstream nodes consume a branch
via `route_name.branch`; a Route carries no `cxl:` and passes all input columns unchanged to every
branch. The migration replaces the transform with a real `route` node and rewires the two outputs
to `route_priority.fulfilled_orders` / `route_priority.priority_report` (see #74 for the diff and
the cross-example audit).

**Field-lineage implication:** today `node_cxl` returns `None` for Route, so
`compute_field_lineage` already treats it as pure passthrough (identity carries of every input
column) — correct for the *node-level* shape. What's missing is **per-branch** modelling: klinx's
`node_input_name` discards the `.branch` port (`pipeline_view.rs:108-113`), so both outputs resolve
to the Route node-level port and the branches are not visually distinct. Closing that gap is the
Route-visualization work below.

## Issue 4 — click-to-select a field → full cross-node lineage (filed as #75)

Distinct from §1.6's transient click-escalation: this is a **persistent selection** (survives
pointer moves) that highlights a field's **full** upstream+downstream lineage across every node it
flows through, dimming the rest — the "details-on-demand, one field at a time" trace. Needs a
`selected_field` signal (precedence over hover), a **transitive** closure helper (distinct from the
1-hop `lineage_closure`, `field_lineage.rs:240-257`), and the same clear-on-view-swap discipline as
hover. Single-select recommended first. See #75.

## Route-node canvas visualization (research → `2026-06-14-route-node-visualization.md`)

Deep-research (25/25 verified claims) on visualizing a conditional Route/Switch node. Strong
cross-tool convergence (n8n, Node-RED, NiFi, Unreal, React Flow): **one output port per branch,
labelled at the port (not the edge), edges bound to a specific branch, and the `default` as a
first-class, always-present, visually-distinct port.** Predicate text → inspector/hover, not inline
(Shneiderman details-on-demand). The user's "filter lineage by route" idea is **highlight /
focus+context** (emphasise the matching branch's path, **dim** non-matching), distinct from
Dataplex *filter* (hide/collapse). Never bundle branch edges; optimise crossing *angle*. Full
recommendations + citations in the companion report.

## Suggested issue split & phasing (filed)

- **#72** (interaction): `FieldEdgeKind` 3-way enum + used/access classifier (headless, unit-tested
  against the `value_tier` fixture) → `HoverTarget`/`pinned` state machine + card/row event ladder
  → `node_carry_edges` helper → 3-colour cable styling. Sub-issue of #64; refines Phase-2 **#67**.
  Open decision: node-hover incoming-only (recommended) vs incoming+outgoing (§1.7).
- **#73** (datatypes): *2a* thread `Type` onto `FieldRow` (Declared + carried, no compile,
  standalone); *2b* emitted types from `CompiledPlan::typed_output_row` / best-effort `type_check`,
  bundled with Phase-3 **#68**. Sub-issue of #64.
- **#74** (Route example): migrate `order_fulfillment.yaml` to `type: route`; audit other examples.
  Standalone.
- **#75** (click-to-select): persistent field selection + transitive full-lineage highlight.
  Sub-issue of #64; builds on #72.
- **Route visualization** (recommend filing as a new sub-issue of #64, overlapping #67's Route
  bullet): per-branch output ports + distinct default port + predicate inspector + route-filtered
  (highlight/dim) lineage. Depends on #74.

---

## Citations

klinx (working tree):
- `crates/klinx/src/pipeline_view/field_lineage.rs` — `FieldEdge`/`passthrough` (44-51),
  `emit_supports` (223-238), `lineage_closure` 1-hop rationale (240-268), EmitEach.source gap
  (226-230), parse-error degrade (143-150).
- `crates/klinx/src/pipeline_view.rs` — `declared_rows` (706-714), derive edges (834-855),
  identity edges (859-871), Source/port origins (930, 995), `field_row_y` (152-154).
- `crates/klinx/src/components/canvas/panel.rs` — `hovered_field` (155), D1 view-swap clear
  (171-202), active-edge compute (210-223), recede group (524-525), background deselect (501-504).
- `crates/klinx/src/components/canvas/node.rs` — row `onmouseenter` (240-246), fields-container
  `onmouseleave` index guard (158-168), collapse clear (131-135).
- `crates/klinx/src/components/canvas/connector.rs` — `FieldConnector` passthrough/derive class
  (60-87).
- `crates/klinx/assets/klinx.css` — field rows/anchors (4221-4281), recede (4293-4296),
  derive/passthrough cable colours (4304-4311), field-name + Enamel colours (4252-4257, 247-255).

clinker engine (git pin `c233a38`):
- `crates/clinker-core/src/config/pipeline_node.rs:722` — `ColumnDecl { ty: cxl::typecheck::Type }`.
- `crates/cxl/src/typecheck/types.rs` — `Type` enum + `display_name`/`Display`.
- `crates/cxl/src/typecheck/pass.rs:133` — `type_check(ResolvedProgram, &Row) -> TypedProgram`.
- `crates/clinker-core/src/plan/compiled.rs:84` — `typed_output_row(name) -> Option<&Row>`.
- `crates/clinker-core/src/plan/bind_schema.rs:2783` — `propagate_row` emit-typing pattern.
- `crates/cxl/src/ast.rs` — `Expr::support_into` (412), `for_each_field_emit` (123),
  `Statement::EmitEach { source, body, .. }` (103).
