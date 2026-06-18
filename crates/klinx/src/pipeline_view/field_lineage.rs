//! Pure field-level lineage analysis for composition canvases.
//!
//! This module is deliberately free of Dioxus: it parses CXL, derives each
//! body node's output field set, and computes the per-field edges that connect
//! a producer column to the consumer column it (partly) derives. The canvas
//! layer renders the results; all of the lineage *logic* lives here so it can
//! be unit-tested headlessly and reasoned about in isolation.
//!
//! Scope is Phase 1 plus the first Phase-2 operators: compositions,
//! transform-precise emit/passthrough rules, Aggregate group-key/output rules,
//! and CXL `let`-chain resolution. Remaining per-node-type precision for
//! route/merge/combine is tracked under Phase 2 (#67); those types stay
//! conservative and never panic.

use std::collections::{HashMap, HashSet};

use cxl::ast::{BinOp, EmitTarget, Expr, LiteralValue, Program, Statement, UnaryOp};
use cxl::builtins::BuiltinRegistry;
use cxl::parser::Parser;
use cxl::typecheck::Type;

/// How a field on a node's output record came to exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FieldKind {
    /// An input/source column declared by a schema (input port or Source).
    ///
    /// The `Default` so [`FieldRow`] can derive `Default`: an origin/declared
    /// column is the natural zero value (the seed kind every other row is
    /// derived from), which lets test literals elide the field via
    /// `..Default::default()`.
    #[default]
    Declared,
    /// A column written by an `emit name = expr` statement.
    Emitted,
    /// An input column carried through unchanged (not shadowed by an emit).
    PassThrough,
}

/// One row in a node's output record: a field name plus how it arose.
///
/// `Default` is derived so test literals can elide the rarely-set fields via
/// `..Default::default()`; the zero value is a [`FieldKind::Declared`] row with
/// no type and no correlation-key/failure-grain role.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FieldRow {
    pub name: String,
    pub kind: FieldKind,
    /// Compact datatype label for inline display (e.g. `float`, `string`,
    /// `int?` for a nullable int). `Some` for [`FieldKind::Declared`] columns
    /// (from a source/port schema) and for [`FieldKind::PassThrough`] columns
    /// that carry a typed producer column through unchanged. For
    /// [`FieldKind::Emitted`] columns it holds the *conservatively inferred*
    /// type of the emit expression ([`infer_emit_type`], #149) — `numeric` for
    /// arithmetic, `bool` for comparisons/logical ops, the literal's type, a
    /// builtin method's return type, or a `let`-chain's resolved type. `None`
    /// when inference is genuinely ambiguous (the liberal Unknown fallback) or
    /// when a producer column has no known type. The inferencer never asserts a
    /// *wrong* type; it falls back to Unknown rather than guess.
    pub ty: Option<String>,
    /// Whether this field is a user-declared driver of a Source's
    /// `correlation_key` (#88). `true` for the [`FieldKind::Declared`] source
    /// columns named in `correlation_key` (a `Single` key marks one field; a
    /// `Compound` key marks each listed field) and for the
    /// [`FieldKind::PassThrough`] rows that carry such a column through a
    /// downstream node unchanged, so the marker follows a CK column along its
    /// lineage. Never `true` for [`FieldKind::Emitted`] rows (a new identity is
    /// not a declared driver) nor for the engine-internal `$ck.<field>` shadow
    /// columns (those are not user-declared and klinx never surfaces them).
    pub is_correlation_key: bool,
    /// The row's lineage precision tier (#148): the *worst* of the precisions of
    /// the lineage edges incident to it, or [`Precision::Unknown`] when the node's
    /// CXL failed [`parse_clean`] so its edges were suppressed entirely (there is
    /// no edge to annotate, so the degradation lives on the row). `Default` is
    /// [`Precision::Exact`] — a clean row with no degraded incident edge — which
    /// keeps test literals eliding the field via `..Default::default()`.
    pub lineage_precision: Precision,
    /// Plain-language reason for `lineage_precision`, surfaced by the Inspector's
    /// per-field precision badge. A `&'static str` (every assigned reason is a fixed
    /// literal, sourced from the row's producing [`FieldEdge::precision_reason`] or
    /// the parse-fail marker), so the row carries no per-field allocation. Empty
    /// (`""`) for the un-degraded `Exact` default and whenever a row folds back to
    /// `Exact`, so `Exact` always means "no degraded reason".
    pub precision_reason: &'static str,
}

/// Whether a [`FieldEdge`] carries (or re-derives) a column's VALUE, or merely
/// *influences* which output values/rows exist without contributing any value.
///
/// Mirrors OpenLineage's `ColumnLineageDatasetFacet` partition: a column edge is
/// either `DIRECT` (the output value is derived from — or identical to — the
/// input value) or `INDIRECT` (the input *influenced* the output — a filter,
/// grouping, join key, or branch condition — but no output value is taken from
/// it). The nature is strictly a function of the edge kind (see
/// [`FieldEdgeKind::nature`]), never an independent field, so an illegal state
/// like a `Direct` join key cannot be represented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeNature {
    /// The output value is taken from (or computed from) the input value: an
    /// identity carry, an accessed carry, or a derive.
    Direct,
    /// The input influenced *which* output rows or values exist — a filter
    /// predicate, a group-by key, a join key, or a branch condition — without
    /// contributing any value to the output.
    Indirect,
}

/// How a [`FieldEdge`] relates its two endpoints — the relationship types the
/// canvas colour-codes (#72, #147).
///
/// The first three variants are all DIRECT (the original `passthrough: bool`
/// split widened by whether a carried `c → c` column *also* feeds a
/// computed/renamed output). The last four are INDIRECT *influence* edges
/// (#147): a control-flow / grouping operator's column influences which output
/// rows survive (or how they are grouped/joined) without deriving any output
/// value. [`FieldEdgeKind::nature`] partitions the variants into
/// [`EdgeNature::Direct`] / [`EdgeNature::Indirect`]; the renderer maps each
/// variant to a distinct stroke style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum FieldEdgeKind {
    /// Identity carry (`c → c`) whose column is read by no derive — it rides
    /// through the node untouched and unreferenced.
    ///
    /// The `Default` variant so [`FieldEdge`] can derive `Default`: a pure
    /// identity carry is the natural zero edge, letting test literals elide the
    /// rarely-varied `kind`/`precision`/`precision_reason` fields via
    /// `..Default::default()`.
    #[default]
    Passthrough,
    /// Identity carry (`c → c`) whose column ALSO appears in some computed or
    /// renamed emit's support on the consumer: carried *and* accessed. The
    /// value still passes through unchanged, but the column is not inert here.
    Access,
    /// An input column feeding an `emit`-produced output field (computed or
    /// renamed), e.g. `line_total → value_tier`. The output value is (re)made
    /// from this input, not carried.
    Derive,
    /// INDIRECT (#147): an input column read by a `Cull` removal predicate
    /// (`drop_group_when`). It decides which groups/rows survive but contributes
    /// no value to any output column. OpenLineage `FILTER`.
    Filter,
    /// INDIRECT (#147): an input column used as an `Aggregate` `group_by` key.
    /// It defines the grouped-record grain — which rows collapse together — but
    /// the grouped output value comes from the aggregate expressions, not the
    /// key's per-row value. OpenLineage `GROUP_BY`. This is the single honest
    /// representation of the post-Aggregate failure grain (it retired the former
    /// `FieldRow::is_aggregate_grain` flag, so the grain is represented once).
    GroupBy,
    /// INDIRECT (#147): an input column read by a `Combine` join predicate
    /// (`where_expr`, e.g. `products.product_code == inventory.product_code`). It
    /// is the real join key on which the inputs are matched — derived from the
    /// predicate's read columns through the same `predicate_support` path as
    /// `Filter`/`Conditional`, so a key whose two sides differ in name
    /// (`left.k1 == right.k2`) is captured and an unrelated same-named column is
    /// not. A `Merge` is a streamwise row UNION (rows stacked, never aligned), so
    /// it performs no join and emits no `JoinKey` edge. Per-input value-column
    /// provenance for the other Combine columns stays out of scope (#67).
    /// OpenLineage `JOIN`.
    JoinKey,
    /// INDIRECT (#147): an input column read by a `Route` branch condition. It
    /// decides which branch a row takes but contributes no value to the routed
    /// output. The default/fallback branch has no predicate, so emits none.
    /// OpenLineage `CONDITIONAL`.
    Conditional,
    /// A composition boundary crossing (#154): an outer field bound to a
    /// composition's inner port (outer input field → inner source port, or inner
    /// output port → outer output field). The value rides across the composition
    /// wall unchanged, so this is a DIRECT crossing — the trace follows it as part
    /// of the value graph — distinguished from intra-scope carries/derives so the
    /// Inspector can mark "↳ enters / ↥ exits composition X". Precision is `Exact`
    /// when bound to a real body row, `Approximate` when the body row is missing
    /// and it degrades to a node-level connector.
    Boundary,
}

impl FieldEdgeKind {
    /// The [`EdgeNature`] of this kind — a pure, total match.
    ///
    /// The carry/derive kinds and the composition `Boundary` crossing are
    /// [`EdgeNature::Direct`] (a value is carried, derived, or ridden across a
    /// composition wall unchanged); the four influence kinds are
    /// [`EdgeNature::Indirect`]. This is the only place nature is decided: deriving
    /// it from the kind (rather than storing it alongside) makes an inconsistent
    /// edge unrepresentable.
    pub fn nature(self) -> EdgeNature {
        match self {
            FieldEdgeKind::Passthrough
            | FieldEdgeKind::Access
            | FieldEdgeKind::Derive
            | FieldEdgeKind::Boundary => EdgeNature::Direct,
            FieldEdgeKind::Filter
            | FieldEdgeKind::GroupBy
            | FieldEdgeKind::JoinKey
            | FieldEdgeKind::Conditional => EdgeNature::Indirect,
        }
    }
}

/// How trustworthy a lineage row or edge is — its graded *precision* tier (#148).
///
/// Provenance theory proves minimal column lineage is undecidable, so every
/// system over-approximates; the differentiator klinx pursues is to *show* where
/// precision degrades rather than silently dropping joins/filters the way dbt and
/// Databricks do. Precision is genuinely orthogonal to [`FieldEdgeKind`] (a
/// `Derive` edge may be `Exact` or `Approximate` depending on whether its support
/// resolved cleanly), so it is its own enum carried alongside `kind`, never folded
/// into it. The accompanying `precision_reason` string explains *why* a row/edge
/// landed in its tier in plain language for the Inspector badge.
///
/// `Default` is `Exact` so [`FieldEdge`] and [`FieldRow`] can derive `Default`:
/// the un-degraded carry is the natural zero, which lets test literals elide the
/// field via `..Default::default()`.
///
/// The variants are declared in ASCENDING degradation order (`Exact` <
/// `Approximate` < `Unknown`), so the derived `Ord` makes `max` select the worst
/// (least precise) tier — the ordering is a compiler-enforced declaration-order
/// invariant, not a hand-maintained rank table. See [`Precision::worst`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Precision {
    /// The lineage is faithful: an identity passthrough/access carry, or a
    /// clean-CXL `Derive` whose entire support resolved to real producer columns
    /// with no `emit each` fan-out. The drawn edge is exactly the dependency.
    #[default]
    Exact,
    /// The lineage is a sound over-approximation: an INDIRECT influence edge
    /// (filter/group-by/join-key/conditional — it influences which rows exist,
    /// not a value), a conservative CXL-less Merge/Combine fan-in carry, or a
    /// `Derive` whose value is fanned out by `emit each` (per-element provenance
    /// is lost). The edge is real but coarser than the true value dependency.
    Approximate,
    /// The lineage could not be computed: the node's CXL failed [`parse_clean`],
    /// so its edges were suppressed (a garbled AST yields wrong edges, worse than
    /// none). `Unknown` therefore lives on the [`FieldRow`] — there is no edge to
    /// annotate — and signals "we genuinely don't know this field's provenance".
    Unknown,
}

impl Precision {
    /// Combine two tiers, keeping the *worse* (less trustworthy) one.
    ///
    /// The order is `Unknown` > `Approximate` > `Exact`: a row incident to any
    /// degraded edge inherits that degradation. A thin `max` over the derived
    /// declaration-order `Ord`. Used to fold a row's precision from its producing
    /// edges and to pick the worst edge when a trace hop is reachable by several.
    pub fn worst(self, other: Precision) -> Precision {
        self.max(other)
    }

    /// The human-facing tier label for the Inspector badge text.
    pub fn precision_label(self) -> &'static str {
        match self {
            Precision::Exact => "exact",
            Precision::Approximate => "approximate",
            Precision::Unknown => "unknown",
        }
    }

    /// The `data-precision` attribute slug — a single CSS-safe token. Kept
    /// distinct from [`Precision::precision_label`] (even though they coincide
    /// today) so a future multi-word label cannot leak whitespace into the
    /// attribute; the slug-safe test guards that contract.
    pub fn precision_attr(self) -> &'static str {
        match self {
            Precision::Exact => "exact",
            Precision::Approximate => "approximate",
            Precision::Unknown => "unknown",
        }
    }
}

/// A field-level lineage edge: `to_node.to_field` is (partly) derived from
/// `from_node.from_field`.
///
/// `kind` ([`FieldEdgeKind`]) distinguishes an identity carry — pure
/// (`Passthrough`) or also-accessed (`Access`) — from a derivation (`Derive`).
/// The canvas renders the three differently so a reader can tell a
/// rename/compute from a carry, and a referenced carry from an inert one.
///
/// `precision` ([`Precision`], #148) grades how faithful the edge is, with
/// `precision_reason` explaining why; both are derived at construction from the
/// edge's kind and support resolution. Precision is deliberately NOT part of edge
/// identity: see the hand-written [`PartialEq`] below.
#[derive(Clone, Debug, Default)]
pub struct FieldEdge {
    pub from_node: usize,
    pub from_field: String,
    pub to_node: usize,
    pub to_field: String,
    pub kind: FieldEdgeKind,
    /// How faithful this edge is (#148). Orthogonal to `kind`; excluded from
    /// [`PartialEq`] / dedup identity so a value carry and its precision annotation
    /// never split one edge into two.
    pub precision: Precision,
    /// Plain-language reason for the `precision` tier, surfaced by the Inspector
    /// badge. A `&'static str` because every constructor assigns a fixed literal
    /// and it is excluded from identity/dedup — so no per-edge `String` allocation
    /// is paid on the hot lineage path. Defaults to `""` for the `Default` edge.
    pub precision_reason: &'static str,
}

/// Identity equality over the 5-tuple `(from_node, from_field, to_node, to_field,
/// kind)` ONLY — `precision`/`precision_reason` are deliberately excluded.
///
/// Two reasons (#148): (1) it keeps `==` and `.contains` in lock-step with the
/// `EdgeAccumulator` dedup key (the same 5-tuple), so an edge that dedups as a
/// duplicate also compares equal; (2) precision is a derived annotation, not part
/// of *what dependency exists*, so two otherwise-identical edges are the same edge
/// regardless of their reason strings. A given identity is always built from the
/// same construction context, so its precision is deterministic anyway.
impl PartialEq for FieldEdge {
    fn eq(&self, other: &Self) -> bool {
        self.from_node == other.from_node
            && self.from_field == other.from_field
            && self.to_node == other.to_node
            && self.to_field == other.to_field
            && self.kind == other.kind
    }
}

impl FieldEdge {
    /// Construct a DIRECT carry edge (`Passthrough`/`Access`), classified
    /// [`Precision::Exact`] — an identity carry reproduces the value verbatim, so
    /// its lineage is faithful (#148). `kind` MUST be a DIRECT carry kind; the
    /// reason names which carry it is.
    pub fn carry(
        from_node: usize,
        from_field: String,
        to_node: usize,
        to_field: String,
        kind: FieldEdgeKind,
    ) -> FieldEdge {
        debug_assert!(
            matches!(kind, FieldEdgeKind::Passthrough | FieldEdgeKind::Access),
            "FieldEdge::carry expects a DIRECT carry kind, got {kind:?}"
        );
        let reason = match kind {
            FieldEdgeKind::Access => "accessed identity carry",
            _ => "identity passthrough",
        };
        FieldEdge {
            from_node,
            from_field,
            to_node,
            to_field,
            kind,
            precision: Precision::Exact,
            precision_reason: reason,
        }
    }

    /// Construct a DIRECT `Derive` edge whose precision depends on how its support
    /// resolved (#148): [`Precision::Exact`] for a clean-CXL derive whose value is
    /// a straight-line function of resolved producer columns, or
    /// [`Precision::Approximate`] when the derive is fanned out by an `emit each`
    /// (its per-element provenance is lost — the edge is real but coarser).
    pub fn derive(
        from_node: usize,
        from_field: String,
        to_node: usize,
        to_field: String,
        fanned: bool,
    ) -> FieldEdge {
        let (precision, reason) = if fanned {
            (
                Precision::Approximate,
                "emit each fan-out loses per-element provenance",
            )
        } else {
            (Precision::Exact, "derived from fully-resolved CXL support")
        };
        FieldEdge {
            from_node,
            from_field,
            to_node,
            to_field,
            kind: FieldEdgeKind::Derive,
            precision,
            precision_reason: reason,
        }
    }

    /// Construct an INDIRECT influence edge (`Filter`/`GroupBy`/`JoinKey`/
    /// `Conditional`), always [`Precision::Approximate`] (#148): an influence edge
    /// states which rows exist or how they group/join, a sound over-approximation
    /// of a value dependency rather than an exact one. `kind` MUST be INDIRECT.
    pub fn influence(
        from_node: usize,
        from_field: String,
        to_node: usize,
        to_field: String,
        kind: FieldEdgeKind,
    ) -> FieldEdge {
        debug_assert_eq!(
            kind.nature(),
            EdgeNature::Indirect,
            "FieldEdge::influence expects an INDIRECT kind, got {kind:?}"
        );
        let reason = match kind {
            FieldEdgeKind::Filter => "INDIRECT filter predicate influence",
            FieldEdgeKind::GroupBy => "INDIRECT group-by grain influence",
            FieldEdgeKind::JoinKey => "INDIRECT join-key influence",
            FieldEdgeKind::Conditional => "INDIRECT branch-condition influence",
            _ => "INDIRECT influence edge",
        };
        FieldEdge {
            from_node,
            from_field,
            to_node,
            to_field,
            kind,
            precision: Precision::Approximate,
            precision_reason: reason,
        }
    }

    /// Construct a conservative CXL-less fan-in carry — a Merge join key carried
    /// from several inputs with no CXL to attribute it — classified
    /// [`Precision::Approximate`] (#148): the column genuinely arrives from each
    /// input, but without CXL we cannot confirm it rides through unchanged, so the
    /// carry is an honest over-approximation rather than an exact passthrough.
    pub fn conservative_fan_in(
        from_node: usize,
        from_field: String,
        to_node: usize,
        to_field: String,
    ) -> FieldEdge {
        FieldEdge {
            from_node,
            from_field,
            to_node,
            to_field,
            kind: FieldEdgeKind::Passthrough,
            precision: Precision::Approximate,
            precision_reason: "conservative CXL-less fan-in carry",
        }
    }

    /// Construct a composition [`FieldEdgeKind::Boundary`] crossing (#154) binding
    /// an outer field to a composition's inner port. `approximate == false` (a real
    /// body row backs the crossing) is [`Precision::Exact`] — the value rides across
    /// the wall unchanged; `approximate == true` (the body row was missing and the
    /// crossing degraded to a node-level connector) is [`Precision::Approximate`].
    pub fn boundary(
        from_node: usize,
        from_field: String,
        to_node: usize,
        to_field: String,
        approximate: bool,
    ) -> FieldEdge {
        let (precision, reason) = if approximate {
            (
                Precision::Approximate,
                "composition boundary degraded: body rows unavailable",
            )
        } else {
            (Precision::Exact, "composition boundary crossing")
        };
        FieldEdge {
            from_node,
            from_field,
            to_node,
            to_field,
            kind: FieldEdgeKind::Boundary,
            precision,
            precision_reason: reason,
        }
    }
}

/// Resolve the let-expanded input-column support of a single `emit`
/// expression's already-collected raw support.
///
/// CXL `let` bindings introduce intermediate names that are *not* input
/// columns: `let w = a + 1.0; emit y = w * 2.0` makes `y` depend on the input
/// column `a`, not on `w`. `support_into` only sees direct references, so it
/// reports `{w}` for `y`. This helper rewrites every raw member that names a
/// `let` into that let's transitively-resolved support, leaving genuine column
/// references untouched. The result is the set of candidate *input columns*
/// the emit truly reads.
///
/// `let_support` maps each `let` name to its own already-resolved support set
/// (built by [`build_let_support`] in declaration order, so a later `let` can
/// reference an earlier one). Cycles cannot occur in well-formed
/// declaration-ordered CXL, but a `visiting` guard makes the walk total even
/// on adversarial input the live canvas may render before validation.
pub fn resolve_support(
    raw: &HashSet<String>,
    let_support: &HashMap<String, HashSet<String>>,
) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut visiting = HashSet::new();
    for name in raw {
        expand_member(name, let_support, &mut visiting, &mut out);
    }
    out
}

/// Expand one raw support member into `out`: if it names a `let`, recurse into
/// that let's resolved support; otherwise it is a real input column.
///
/// `visiting` breaks cycles: a `let` already on the active path is treated as
/// a leaf so the recursion always terminates.
fn expand_member(
    name: &str,
    let_support: &HashMap<String, HashSet<String>>,
    visiting: &mut HashSet<String>,
    out: &mut HashSet<String>,
) {
    match let_support.get(name) {
        Some(_) if visiting.contains(name) => {
            // Cycle guard: stop expanding a let already on the path. It is not
            // an input column, so contributing nothing here is correct.
        }
        Some(inner) => {
            visiting.insert(name.to_string());
            for member in inner {
                expand_member(member, let_support, visiting, out);
            }
            visiting.remove(name);
        }
        None => {
            // Not a let binding → a genuine input-column reference.
            out.insert(name.to_string());
        }
    }
}

/// Build the per-`let` resolved support map for a parsed program.
///
/// Walks top-level `Statement::Let` in declaration order. Each let's raw
/// support is collected via `Expr::support_into`, then immediately resolved
/// against the lets declared before it (an earlier `let` referenced by a later
/// one is expanded to its own input columns). The returned map therefore holds
/// each let's *input-column* support, ready for [`resolve_support`] to expand
/// emit expressions.
pub fn build_let_support(program: &Program) -> HashMap<String, HashSet<String>> {
    let mut let_support: HashMap<String, HashSet<String>> = HashMap::new();
    for stmt in &program.statements {
        if let Statement::Let { name, expr, .. } = stmt {
            let mut raw = HashSet::new();
            expr.support_into(&mut raw);
            // Resolve against lets declared *earlier*; `let_support` does not
            // yet contain this binding, so a self-reference falls through as a
            // column (it cannot reference itself in declaration-ordered CXL).
            let resolved = resolve_support(&raw, &let_support);
            let_support.insert(name.to_string(), resolved);
        }
    }
    let_support
}

/// Parse a body node's CXL, returning the program only when it parses cleanly.
///
/// `Parser::parse` is lenient — it returns a (possibly partial) AST alongside
/// collected errors. For lineage we require an error-free parse: a partial AST
/// can silently drop or mangle statements, which would produce *wrong* edges
/// (worse than none). On any error we return `None` so the caller renders the
/// node's fields but skips its lineage edges, per the spec's
/// never-panic / degrade-gracefully rule.
pub fn parse_clean(cxl: &str) -> Option<Program> {
    let result = Parser::parse(cxl);
    if result.errors.is_empty() {
        Some(result.ast)
    } else {
        None
    }
}

/// A short, lowercase datatype label for inline display on a field row, e.g.
/// `float`, `string`, `datetime`, and `int?` for `Nullable(Int)`. The engine's
/// `Display`/`display_name` are unsuitable: `display_name` drops the inner type
/// of `Nullable`, and `Display` renders the verbose `Nullable(Int)` form.
///
/// Lives here (rather than in the canvas layer) so the lineage core can stamp an
/// inferred emit type ([`infer_emit_type`], #149) onto an [`FieldRow`] without a
/// round-trip through the renderer.
pub fn compact_type(ty: &Type) -> String {
    match ty {
        Type::Nullable(inner) => format!("{}?", compact_type(inner)),
        Type::Null => "null".to_string(),
        Type::Bool => "bool".to_string(),
        Type::Int => "int".to_string(),
        Type::Float => "float".to_string(),
        Type::String => "string".to_string(),
        Type::Date => "date".to_string(),
        Type::DateTime => "datetime".to_string(),
        Type::Array => "array".to_string(),
        Type::Map => "map".to_string(),
        Type::Numeric => "numeric".to_string(),
        Type::Any => "any".to_string(),
    }
}

/// The process-wide builtin-method registry, built once.
///
/// `BuiltinRegistry::new()` constructs a ~50-entry method table (each entry
/// allocating a `Vec`); the table is static data, so rebuilding it per node on
/// every canvas re-derive (the view is recomputed on each CXL keystroke) is pure
/// waste. One shared instance behind a `OnceLock` removes it.
fn builtin_registry() -> &'static BuiltinRegistry {
    static REGISTRY: std::sync::OnceLock<BuiltinRegistry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(BuiltinRegistry::new)
}

/// Whether `ty`'s base (nullable stripped) is a *known* numeric type.
fn is_known_numeric(ty: &Type) -> bool {
    matches!(
        ty.unwrap_nullable(),
        Type::Int | Type::Float | Type::Numeric
    )
}

/// Whether `ty`'s base is a *known* string type.
fn is_known_string(ty: &Type) -> bool {
    matches!(ty.unwrap_nullable(), Type::String)
}

/// Whether `ty`'s base is a *known* type that arithmetic cannot operate on
/// (bool/date/array/map). `Any`, `Null`, numerics and strings are excluded — the
/// caller handles those separately.
fn is_known_non_arithmetic(ty: &Type) -> bool {
    matches!(
        ty.unwrap_nullable(),
        Type::Bool | Type::Date | Type::DateTime | Type::Array | Type::Map
    )
}

/// Result type of CXL `+`, which is overloaded between numeric addition and
/// string concatenation.
///
/// Conservative: `String` only when both sides are known strings; `Numeric` only
/// when a side is known numeric and neither side is a known non-numeric; `Any`
/// otherwise — crucially including the both-unknown case, where the operator
/// could be either an add or a concat and `Numeric` would be a guess (#149).
fn infer_add_type(lt: &Type, rt: &Type) -> Type {
    if is_known_string(lt) && is_known_string(rt) {
        Type::String
    } else if is_known_string(lt)
        || is_known_string(rt)
        || is_known_non_arithmetic(lt)
        || is_known_non_arithmetic(rt)
    {
        // A string mixed with a non-string, or any non-arithmetic operand: the
        // engine rejects it, so we have no honest concrete type.
        Type::Any
    } else if is_known_numeric(lt) || is_known_numeric(rt) {
        Type::Numeric
    } else {
        // Both operands unknown: could be a numeric add or a string concat.
        Type::Any
    }
}

/// Result type of CXL `-`, `*`, `/`, `%` — numeric-only operators. `Numeric`
/// unless an operand is a known non-numeric type (string/bool/date/…), in which
/// case the engine would reject it and we return `Any`. Two unknown operands
/// still yield `Numeric`: these operators have no non-numeric overload, so the
/// only valid typing of `a * b` is numeric.
fn infer_numeric_binop_type(lt: &Type, rt: &Type) -> Type {
    if is_known_string(lt)
        || is_known_string(rt)
        || is_known_non_arithmetic(lt)
        || is_known_non_arithmetic(rt)
    {
        Type::Any
    } else {
        Type::Numeric
    }
}

/// Conservatively infer the datatype of an `emit` expression *without* the engine
/// typechecker (#149).
///
/// Mirrors the subset of the engine's own rules (`cxl/src/typecheck/pass.rs`)
/// that can be decided from expression *shape* alone, and returns [`Type::Any`]
/// — the liberal "Unknown" — for everything else. The guarantee is conservatism:
/// the result is either a type the engine would also assign (possibly a wider
/// supertype, e.g. `Numeric` where the engine resolves `Int`) or `Any`; it never
/// asserts a type the engine would contradict.
///
/// Covered shapes:
/// - literals → their concrete type;
/// - arithmetic — operand-aware, because CXL `+` is overloaded:
///   - `*`, `-`, `/`, `%` → [`Type::Numeric`] unless an operand is a *known*
///     non-numeric type (then `Any` — the engine would reject it);
///   - `+` → `String` when both operands are known strings (concatenation),
///     `Numeric` when at least one operand is known numeric (and none is a known
///     non-numeric), and `Any` when both operands are unknown (it could be
///     either a numeric add or a string concat — `Numeric` would be a guess);
/// - comparisons and logical ops → [`Type::Bool`];
/// - unary `!` → `Bool`, unary `-` → its operand's inferred type;
/// - method calls → the builtin's declared return type (covers string methods);
/// - a reference to a `let` binding → that binding's inferred type
///   (`let_types`), resolving `let`-chains transitively.
///
/// Bare input-column references are `Any` in raw mode (their type needs the
/// source schema, which lives outside this pure analysis), as is any uncovered
/// shape (conditionals, aggregates, window calls, …).
fn infer_emit_type(
    expr: &Expr,
    let_types: &HashMap<String, Type>,
    builtins: &BuiltinRegistry,
) -> Type {
    match expr {
        Expr::Literal { value, .. } => match value {
            LiteralValue::Int(_) => Type::Int,
            LiteralValue::Float(_) => Type::Float,
            LiteralValue::String(_) => Type::String,
            LiteralValue::Bool(_) => Type::Bool,
            LiteralValue::Date(_) => Type::Date,
            LiteralValue::Null => Type::Null,
        },
        Expr::Binary { op, lhs, rhs, .. } => match op {
            BinOp::Add => {
                let lt = infer_emit_type(lhs, let_types, builtins);
                let rt = infer_emit_type(rhs, let_types, builtins);
                infer_add_type(&lt, &rt)
            }
            BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                let lt = infer_emit_type(lhs, let_types, builtins);
                let rt = infer_emit_type(rhs, let_types, builtins);
                infer_numeric_binop_type(&lt, &rt)
            }
            BinOp::Eq
            | BinOp::Neq
            | BinOp::Gt
            | BinOp::Lt
            | BinOp::Gte
            | BinOp::Lte
            | BinOp::And
            | BinOp::Or => Type::Bool,
        },
        Expr::Unary { op, operand, .. } => match op {
            UnaryOp::Not => Type::Bool,
            UnaryOp::Neg => infer_emit_type(operand, let_types, builtins),
        },
        // The engine resolves a method's return type through this same registry,
        // so a known method agrees with the engine; an unknown method is `Any`.
        Expr::MethodCall { method, .. } => builtins
            .lookup_method(method.as_ref())
            .map(|def| Type::from_type_tag(def.return_type))
            .unwrap_or(Type::Any),
        // A reference resolves to a `let` binding's inferred type when it names
        // one; a bare input column has no known type here, so it stays `Any`.
        Expr::FieldRef { name, .. } => let_types.get(name.as_ref()).cloned().unwrap_or(Type::Any),
        // Everything else (qualified refs, conditionals, aggregates, window
        // calls, …) is left Unknown — the conservative fallback.
        _ => Type::Any,
    }
}

/// The inferred type of every top-level `let` binding, in declaration order.
///
/// The type analogue of [`build_let_support`]: each `let`'s expression is typed
/// against the bindings declared *before* it, so a chain
/// `let u = a + 1.0; let v = u; emit y = v` resolves `v` (and thus `y`) through
/// `u`'s inferred type. Declaration-ordered CXL cannot forward-reference, so a
/// not-yet-seen name simply infers as `Any`.
fn build_let_types(program: &Program, builtins: &BuiltinRegistry) -> HashMap<String, Type> {
    let mut let_types: HashMap<String, Type> = HashMap::new();
    for stmt in &program.statements {
        if let Statement::Let { name, expr, .. } = stmt {
            let ty = infer_emit_type(expr, &let_types, builtins);
            let_types.insert(name.to_string(), ty);
        }
    }
    let_types
}

/// The inferred type of every field-emit target, keyed by name.
///
/// Descends `emit each` bodies via `for_each_field_emit` so fanned-out fields are
/// typed too. A name emitted more than once keeps its *last* type — CXL record
/// semantics overwrite an earlier same-named emit, so the final value's type is
/// the last writer's.
fn emit_target_types(
    program: &Program,
    let_types: &HashMap<String, Type>,
    builtins: &BuiltinRegistry,
) -> HashMap<String, Type> {
    let mut types: HashMap<String, Type> = HashMap::new();
    cxl::ast::for_each_field_emit(&program.statements, &mut |name, expr| {
        types.insert(name.to_string(), infer_emit_type(expr, let_types, builtins));
    });
    types
}

/// Compute a transform-like node's output field rows from its input columns
/// and parsed CXL.
///
/// `input_cols` is the ordered union of the node's predecessors' output field
/// names (declaration order, de-duplicated). The output is:
///   1. every input column **not** shadowed by an emit, as `PassThrough`
///      (in input order), then
///   2. every field-emit target, as `Emitted` (in emit order).
///
/// This mirrors CXL record semantics: an `emit name = …` overwrites the
/// same-named input column (so it appears once, as `Emitted`), while unmatched
/// input columns ride through unchanged.
pub fn transform_output_fields(input_cols: &[String], program: &Program) -> Vec<FieldRow> {
    let emitted = emitted_field_names(program);
    let emitted_set: HashSet<&str> = emitted.iter().map(|s| s.as_str()).collect();

    // An emit that just re-emits a column unchanged (`emit c = c` or
    // `emit c = src.c`) is a passthrough, NOT a created/altered field — so it is
    // classified `PassThrough`, not `Emitted`. This matters for nodes like a join
    // that re-`emit` every input column: those columns are carried, so they read
    // as passthroughs and keep their datatypes, instead of all looking computed.
    let copies = emit_copy_targets(program, input_cols);

    // Emitted-row datatypes are inferred here (#149); PassThrough datatypes are
    // filled in by the caller (`compute_field_lineage`) from the producer column.
    // The builtin registry is process-wide; the let-type map is per node.
    let builtins = builtin_registry();
    let let_types = build_let_types(program, builtins);
    let emit_types = emit_target_types(program, &let_types, builtins);

    let mut rows: Vec<FieldRow> = Vec::new();
    for col in input_cols {
        if !emitted_set.contains(col.as_str()) {
            rows.push(FieldRow {
                name: col.clone(),
                kind: FieldKind::PassThrough,
                ty: None,
                // The correlation-key flag is stamped onto carried passthrough
                // rows by the caller (`compute_field_lineage`), mirroring the
                // datatype carry; it cannot be known from CXL alone.
                ..Default::default()
            });
        }
    }
    for name in emitted {
        let kind = if copies.contains(&name) {
            FieldKind::PassThrough
        } else {
            FieldKind::Emitted
        };
        // A computed Emitted row shows its inferred type, dropping the liberal
        // `Any` Unknown to `None` (no label). An identity-copy row is a
        // PassThrough whose type the caller carries from its producer, so leave
        // it `None` here.
        let ty = match kind {
            FieldKind::Emitted => emit_types
                .get(&name)
                .filter(|t| **t != Type::Any)
                .map(compact_type),
            _ => None,
        };
        rows.push(FieldRow {
            name,
            kind,
            ty,
            ..Default::default()
        });
    }
    rows
}

/// Output rows for an Aggregate whose CXL did not parse cleanly.
///
/// The aggregate's `group_by` list is normal YAML config, not inferred from the
/// CXL body, so it can still be shown safely when emit extraction is unavailable.
/// Repeated keys keep their first slot.
pub fn aggregate_group_key_output_fields(group_by: &[String]) -> Vec<FieldRow> {
    let mut rows = Vec::with_capacity(group_by.len());
    let mut seen = HashSet::new();
    for key in group_by {
        if seen.insert(key.as_str()) {
            rows.push(FieldRow {
                name: key.clone(),
                kind: FieldKind::PassThrough,
                ty: None,
                ..Default::default()
            });
        }
    }
    rows
}

/// Output rows for an Aggregate with parseable CXL.
///
/// Aggregates produce a new grouped record. Its raw-mode row shape is therefore
/// the configured group keys first, then aggregate `emit` targets. Unlike
/// [`transform_output_fields`], an identity-looking `emit c = c` is still an
/// aggregate emit target rather than a pass-through row, because the input row
/// was reduced into a grouped output record.
pub fn aggregate_output_fields(group_by: &[String], program: &Program) -> Vec<FieldRow> {
    let mut rows = aggregate_group_key_output_fields(group_by);
    let mut seen: HashSet<String> = rows.iter().map(|row| row.name.clone()).collect();
    for name in emitted_field_names(program) {
        if seen.insert(name.clone()) {
            rows.push(FieldRow {
                name,
                kind: FieldKind::Emitted,
                ty: None,
                ..Default::default()
            });
        }
    }
    rows
}

/// Emit targets that are pure identity copies of an **input column** — i.e.
/// `emit c = c` or `emit c = src.c` (a qualified reference whose final field is
/// `c`), where `c` is one of `input_cols`. Such an emit re-emits a column
/// unchanged, so it is a passthrough rather than a created/altered (computed)
/// field. Restricting to `input_cols` excludes a same-named `let` binding and a
/// rename (`emit y = x`, which produces a genuinely new column).
pub fn emit_copy_targets(program: &Program, input_cols: &[String]) -> HashSet<String> {
    let inputs: HashSet<&str> = input_cols.iter().map(|s| s.as_str()).collect();
    let mut copies = HashSet::new();
    cxl::ast::for_each_field_emit(&program.statements, &mut |name, expr| {
        if bare_field_ref(expr) == Some(name) && inputs.contains(name) {
            copies.insert(name.to_string());
        }
    });
    copies
}

/// The field name an expression references when it is *exactly* a column
/// reference (`c` or `src.c` — the final dotted part); `None` for any computed
/// expression (a binary op, method call, literal, conditional, …).
fn bare_field_ref(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::FieldRef { name, .. } => Some(name),
        Expr::QualifiedFieldRef { parts, .. } => parts.last().map(|p| p.as_ref()),
        _ => None,
    }
}

/// Output field rows for a node with no parseable CXL (or a non-emitting
/// type): every input column carried through unchanged.
///
/// Used both for nodes whose CXL failed to parse (fields still render, edges
/// skipped) and for Phase-1 best-effort handling of types without precise
/// emit rules.
pub fn passthrough_output_fields(input_cols: &[String]) -> Vec<FieldRow> {
    // Types are filled in by the caller from each column's producer (see
    // `transform_output_fields`).
    input_cols
        .iter()
        .map(|col| FieldRow {
            name: col.clone(),
            kind: FieldKind::PassThrough,
            ty: None,
            // CK flag stamped onto carried rows by the caller (see
            // `transform_output_fields`).
            ..Default::default()
        })
        .collect()
}

/// The ordered, de-duplicated list of `emit name = …` field targets in a
/// program, descending into `emit each` bodies via `for_each_field_emit`.
///
/// Order is first-seen emit order; a name emitted twice keeps its first slot.
fn emitted_field_names(program: &Program) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    cxl::ast::for_each_field_emit(&program.statements, &mut |name, _expr| {
        if seen.insert(name.to_string()) {
            names.push(name.to_string());
        }
    });
    names
}

/// Per-`emit` let-resolved input-column support for a program, in emit order.
///
/// Returns `(emit_target, resolved_support)` pairs: the field a statement
/// writes and the set of input columns its expression reads after let-chain
/// resolution. The canvas turns each pair into derive edges by intersecting
/// the support with the node's real input columns.
///
/// A field emitted inside an `emit each` / `emit each … outer` block additionally
/// depends on the iterated **source** column(s) — the fan-out binds each element
/// of `source` to the loop variable, so every body emit is derived from `source`
/// (#150). Without this a fanned-out output would lose its upstream derive edge
/// entirely. Nested fan-out accumulates: an inner body emit depends on every
/// enclosing source.
pub fn emit_supports(program: &Program) -> Vec<(String, HashSet<String>)> {
    let let_support = build_let_support(program);
    let mut out: Vec<(String, HashSet<String>)> = Vec::new();
    collect_emit_supports(&program.statements, &let_support, &HashSet::new(), &mut out);
    out
}

/// The set of `emit` targets produced *inside* an `emit each` / `explode outer`
/// fan-out body (#148 precision).
///
/// A fanned-out field's value is computed per iterated element, so its provenance
/// is genuinely coarser than a straight-line emit: the upstream→field derive is
/// real but the per-element correspondence is lost. Edges into such a target are
/// therefore classified [`Precision::Approximate`]. Membership is recorded
/// regardless of whether the iterated source resolves to any column — the fan-out
/// itself is the source of imprecision, not the source's column support — so a
/// `$pipeline.items` source (empty support) still marks its body emits fanned.
///
/// The set is keyed by field NAME, not by emit statement, and that is deliberate
/// (#148 S1): a field emitted BOTH inside a fan-out body AND by a straight-line
/// emit on the same node is reported fanned, degrading every derive edge into that
/// field to Approximate — even the straight-line one. Its final value is the union
/// of both emits, so part of its provenance IS fan-derived; marking the whole
/// field Approximate errs toward over-approximation, the SAFE direction (we never
/// claim Exact for a field whose value is partly per-element). Statement-scoped
/// degradation would need to thread emit-statement identity through `emit_supports`
/// and the edge builder and to define a precision for a column whose value mixes
/// fanned and non-fanned emits — not worth the plumbing for the safe-direction
/// gain. See `emit_each_fanned_targets_is_conservative_per_field` for the locked
/// behaviour.
pub fn emit_each_fanned_targets(program: &Program) -> HashSet<String> {
    let mut fanned = HashSet::new();
    collect_fanned_targets(&program.statements, false, &mut fanned);
    fanned
}

/// Recursive worker for [`emit_each_fanned_targets`]: collect every field-emit
/// target reached while `inside` an `emit each` / `explode outer` body.
fn collect_fanned_targets(stmts: &[Statement], inside: bool, fanned: &mut HashSet<String>) {
    for stmt in stmts {
        match stmt {
            Statement::Emit {
                name,
                target: EmitTarget::Field,
                ..
            } if inside => {
                fanned.insert(name.to_string());
            }
            Statement::EmitEach { body, .. } | Statement::ExplodeOuter { body, .. } => {
                collect_fanned_targets(body, true, fanned);
            }
            _ => {}
        }
    }
}

/// The input-column support of a standalone *predicate* expression — the columns
/// a `Cull` removal predicate, a `Route` branch condition, or a `Combine`
/// `where_expr` join predicate reads (#147).
///
/// INDIRECT influence edges originate from predicate expression *strings* (e.g.
/// `count(*) < 2`, `region == 'EU'`, `a.id == b.id`), which are not `emit`
/// statements and so `parse_clean` cannot parse on their own. This wraps the
/// predicate as a synthetic `emit __pred = {expr}` statement and reuses the
/// entire `parse_clean` + [`emit_supports`] machinery, returning the single
/// resulting support set: the input columns the predicate truly reads. A
/// predicate is structurally a single expression, so it cannot carry a
/// statement-level `let`; the shared let-resolution pass is therefore a no-op for
/// any well-formed predicate (it only fires for multi-statement `emit` bodies).
///
/// Returns `None` when the wrapped statement does not parse cleanly (mirroring
/// the module-wide never-infer-from-garbage rule — a predicate we cannot trust
/// to parse must yield no edges rather than wrong ones) or when no field emit is
/// recovered. A predicate over only literals/constants parses cleanly and yields
/// an empty set (no producer columns, hence no edges), which is distinct from
/// the `None` parse-failure case.
pub fn predicate_support(expr: &str) -> Option<HashSet<String>> {
    let program = parse_clean(&format!("emit __pred = {expr}"))?;
    let mut supports = emit_supports(&program);
    // The wrap is trusted ONLY when it parsed into EXACTLY the single synthetic
    // `__pred` field emit we authored. A pathological `expr` could parse into a
    // body with extra field emits (e.g. an injected statement) or none at all
    // (e.g. it parsed as something other than a `__pred` field emit); in either
    // case we never-infer-from-garbage and return `None` rather than read a
    // support set we cannot trust. (Exactly one entry whose target is `__pred`.)
    match supports.pop() {
        Some((name, support)) if name == "__pred" && supports.is_empty() => Some(support),
        _ => None,
    }
}

/// Recursive worker for [`emit_supports`], mirroring
/// `cxl::ast::for_each_field_emit`'s `EmitTarget::Field` filtering while
/// threading each enclosing `emit each` source's resolved support down onto the
/// body emits it fans out (#150).
fn collect_emit_supports(
    stmts: &[Statement],
    let_support: &HashMap<String, HashSet<String>>,
    enclosing_source_support: &HashSet<String>,
    out: &mut Vec<(String, HashSet<String>)>,
) {
    for stmt in stmts {
        match stmt {
            Statement::Emit {
                name,
                expr,
                target: EmitTarget::Field,
                ..
            } => {
                let mut raw = HashSet::new();
                expr.support_into(&mut raw);
                let mut resolved = resolve_support(&raw, let_support);
                // The fanned-out field is derived from the iterated source
                // column(s) of every enclosing `emit each`, in addition to its
                // own expression's support.
                resolved.extend(enclosing_source_support.iter().cloned());
                out.push((name.to_string(), resolved));
            }
            Statement::EmitEach { source, body, .. }
            | Statement::ExplodeOuter { source, body, .. } => {
                // Resolve this fan-out's source support and union it with any
                // outer enclosing sources so nested fan-out accumulates. A
                // literal or empty source resolves to an empty set, adding no
                // spurious edges.
                let mut src_raw = HashSet::new();
                source.support_into(&mut src_raw);
                let mut nested = enclosing_source_support.clone();
                nested.extend(resolve_support(&src_raw, let_support));
                collect_emit_supports(body, let_support, &nested, out);
            }
            _ => {}
        }
    }
}

/// Group a set of lineage edges' `(node, field)` endpoints by node.
///
/// For every edge in `edges`, both its `from` side (`(from_node, from_field)`)
/// and its `to` side (`(to_node, to_field)`) are recorded under that node's
/// index. The result maps each node index to the de-duplicated, sorted list of
/// its endpoint field names. The canvas uses it to tint the individual field-row
/// cells that are lineage endpoints of the active hover, so a reader of a
/// multi-field node sees *which row* is the source/target — not just which card
/// participates (the existing whole-node dim already conveys the latter).
///
/// The caller passes only the edges whose cable anchors actually RESOLVE, so a
/// highlighted cell can never appear on a dimmed, cable-less card: the highlight,
/// the dim exemption, and the drawn cable all derive from the same resolved-edge
/// set. Names are sorted purely for determinism (stable per-node `Vec` across
/// renders, so `CanvasNode`'s `PartialEq` memoization is not defeated by set
/// iteration order); a self-loop edge contributes its endpoint once because the
/// per-node accumulation de-duplicates.
pub fn group_endpoints_by_node<'a>(
    edges: impl IntoIterator<Item = &'a FieldEdge>,
) -> HashMap<usize, Vec<String>> {
    let mut by_node: HashMap<usize, HashSet<String>> = HashMap::new();
    for edge in edges {
        by_node
            .entry(edge.from_node)
            .or_default()
            .insert(edge.from_field.clone());
        by_node
            .entry(edge.to_node)
            .or_default()
            .insert(edge.to_field.clone());
    }
    by_node
        .into_iter()
        .map(|(node, names)| {
            let mut names: Vec<String> = names.into_iter().collect();
            names.sort_unstable();
            (node, names)
        })
        .collect()
}

/// The DIRECT (1-hop) lineage neighbourhood of one `(node, field)` anchor over a
/// field-edge set.
///
/// Returns the indices (into `edges`) of every edge *incident* to the anchor —
/// every edge whose source OR target is exactly `(node, field)`, in EITHER
/// direction, INCLUDING passthrough carries. There is deliberately NO transitive
/// walk and NO edge-kind filter.
///
/// WHY 1-hop, both kinds: hovering a field should reveal its immediate
/// neighbourhood — the producers it reads from and the consumers it feeds at the
/// ADJACENT nodes — so a reader sees a passthrough column's 1:1 carry to the next
/// node alongside any derive it participates in. A transitive walk re-creates the
/// "light up half the graph" failure mode (a passthrough column threads through
/// every node, so following carries floods the closure); a 1-hop neighbourhood
/// stays locally legible. Including passthrough edges is the FIX (#70 follow-up):
/// the prior derive-only walk hid the 1:1 carries the user wanted to see.
///
/// The canvas draws exactly these edges on hover and dims the rest.
pub fn lineage_closure(edges: &[FieldEdge], node: usize, field: &str) -> HashSet<usize> {
    edges
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            (e.from_node == node && e.from_field == field)
                || (e.to_node == node && e.to_field == field)
        })
        .map(|(i, _)| i)
        .collect()
}

/// The CARRY edges incident to a whole node — its node-scope hover reveal (#72).
///
/// The FULL pipeline lineage of one `(node, field)` anchor — its complete
/// transitive provenance AND impact across the whole pipeline (#75 click).
///
/// Where [`lineage_closure`] is deliberately 1-hop (the hover reveal, kept
/// locally legible), this is the CLICK-to-select reveal: it walks the column's
/// entire directed lineage — every upstream edge it (transitively) derives from
/// or is carried from, UNION every downstream edge that (transitively) carries
/// or derives from it. Returns the indices (into `edges`) of every edge on those
/// paths.
///
/// The walk is DIRECTED (ancestors via `to → from`, descendants via
/// `from → to`), not an undirected connected-component flood: a sibling column
/// that merely shares a downstream consumer with the anchor is NOT pulled in
/// unless it is itself up- or down-stream of the anchor. This keeps "select a
/// column" to that column's real lineage. Drawing a whole column's transitive
/// lineage at once is permitted ONLY on explicit click (one column on demand) —
/// hover stays 1-hop to avoid flooding the canvas.
pub fn field_lineage_full(edges: &[FieldEdge], node: usize, field: &str) -> HashSet<usize> {
    field_lineage_full_capped(edges, node, field, None)
}

/// [`field_lineage_full`] with a deterministic per-direction hop cap (#123).
///
/// `hop_cap` bounds how many edges deep each directed walk (provenance and
/// impact) is allowed to expand. `None` is the uncapped walk — identical to
/// [`field_lineage_full`], which delegates here with `None`. `Some(n)` keeps only
/// the edges reachable within `n` hops of the anchor in each direction:
///  - `Some(0)` yields the empty set (no edge is within zero hops);
/// - `Some(1)` yields exactly the anchor's directly-incident edges in both
///   directions (the 1-hop neighbourhood, like [`lineage_closure`]);
/// - each larger `n` is a SUPERSET of the previous (the bounded set grows
///   monotonically), so the canvas "expand further" affordance can raise the cap
///   and only ever reveal more, never reshuffle what was already shown.
///
/// The bound is on edge DEPTH (the BFS layer at which an edge is first crossed),
/// not on endpoint count, so a large fan-out at one depth does not consume the
/// budget. The walk is breadth-first per depth layer with a per-direction visited
/// set, so the same `(edges, node, field, hop_cap)` always yields the same set —
/// the determinism the cap-level tests assert.
///
/// The walk stays KIND-AGNOSTIC (the canvas value/influence toggle is deferred to
/// PR5's dual value/influence ribbon) so INDIRECT edges are never permanently
/// invisible on the canvas; the Inspector trace tree carries its own INDIRECT
/// include/exclude toggle (#153), scoped to that surface.
pub fn field_lineage_full_capped(
    edges: &[FieldEdge],
    node: usize,
    field: &str,
    hop_cap: Option<usize>,
) -> HashSet<usize> {
    let mut result: HashSet<usize> = HashSet::new();

    // Upstream (provenance): follow edges INTO the current endpoint, back to
    // origins. Downstream (impact): follow edges OUT to sinks. Each direction is
    // its own breadth-first walk over the (node, field) endpoint graph, with its
    // own visited set; both deposit edge indices into the shared `result`.
    //
    // The frontier is drained one full DEPTH LAYER at a time (`depth` counts the
    // layers already crossed), so `hop_cap` can stop expansion at a precise edge
    // depth — a depth-first pop order would make the cap order-sensitive.
    for forward in [false, true] {
        let mut seen: HashSet<(usize, String)> = HashSet::new();
        let start = (node, field.to_string());
        seen.insert(start.clone());
        let mut frontier = vec![start];
        let mut depth = 0usize;
        while !frontier.is_empty() {
            if hop_cap.is_some_and(|cap| depth >= cap) {
                break;
            }
            let mut next_frontier: Vec<(usize, String)> = Vec::new();
            for (n, f) in frontier.drain(..) {
                for (i, e) in edges.iter().enumerate() {
                    // Forward walk steps producer→consumer (match the `from` side
                    // and hop to `to`); backward walk steps consumer→producer.
                    let next = if forward && e.from_node == n && e.from_field == f {
                        Some((e.to_node, e.to_field.clone()))
                    } else if !forward && e.to_node == n && e.to_field == f {
                        Some((e.from_node, e.from_field.clone()))
                    } else {
                        None
                    };
                    if let Some(other) = next {
                        result.insert(i);
                        if seen.insert(other.clone()) {
                            next_frontier.push(other);
                        }
                    }
                }
            }
            frontier = next_frontier;
            depth += 1;
        }
    }
    result
}

/// The Filter-mode keep-set of NODE indices for an active reveal (#123).
///
/// Given the participating field-edge indices of a reveal (the closure from
/// [`field_lineage_full`]/[`field_lineage_full_capped`] or [`lineage_closure`])
/// and the anchor node, returns every node index that Filter mode must KEEP
/// visible: the anchor plus both endpoints of every participating edge. A node not
/// in this set is off-path clutter that Filter mode hides.
///
/// WHY endpoints-of-participating-edges suffices to avoid dangling partial paths:
/// the closure walks the anchor's full transitive up+down lineage, so every node
/// on a connecting path between two kept nodes is itself an endpoint of some
/// participating edge — there are no gaps to bridge. An edge is therefore safe to
/// draw iff BOTH its endpoints are in this set; the caller filters node-level
/// connectors on exactly that predicate, so no half-edge can dangle to a hidden
/// card. The anchor is included even when it has no resolved edges (a leaf column
/// selected on a single-node graph) so the selected card never hides itself.
pub fn lineage_keep_nodes<'a>(
    participating_edges: impl IntoIterator<Item = &'a FieldEdge>,
    anchor_node: usize,
) -> HashSet<usize> {
    let mut keep: HashSet<usize> = HashSet::new();
    keep.insert(anchor_node);
    for edge in participating_edges {
        keep.insert(edge.from_node);
        keep.insert(edge.to_node);
    }
    keep
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(src: &str) -> Program {
        parse_clean(src).expect("fixture CXL parses cleanly")
    }

    fn cols(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    /// `nature()` partitions EVERY variant: the three carry/derive kinds are
    /// DIRECT, the four influence kinds are INDIRECT. Listing every variant by
    /// name (no wildcard) makes this test fail to compile if a future variant is
    /// added without a nature decision — the partition can never silently drift.
    #[test]
    fn edge_kind_nature_partition_is_exhaustive() {
        for kind in [
            FieldEdgeKind::Passthrough,
            FieldEdgeKind::Access,
            FieldEdgeKind::Derive,
            FieldEdgeKind::Boundary,
        ] {
            assert_eq!(kind.nature(), EdgeNature::Direct, "{kind:?} must be Direct");
        }
        for kind in [
            FieldEdgeKind::Filter,
            FieldEdgeKind::GroupBy,
            FieldEdgeKind::JoinKey,
            FieldEdgeKind::Conditional,
        ] {
            assert_eq!(
                kind.nature(),
                EdgeNature::Indirect,
                "{kind:?} must be Indirect"
            );
        }
    }

    /// A predicate over real columns resolves to exactly those input columns,
    /// reusing the let-resolution machinery (the `count(*)` aggregate adds no
    /// column; `region` and `amount` are the real reads).
    #[test]
    fn predicate_support_extracts_read_columns() {
        let support = predicate_support("region == 'EU' and amount > 10.0").unwrap();
        assert_eq!(
            support,
            HashSet::from(["region".to_string(), "amount".to_string()])
        );
    }

    /// A single-column predicate resolves to exactly that column. A predicate is
    /// structurally an expression, so it cannot carry a statement-level `let`;
    /// this locks the simplest read-column extraction (no let resolution to
    /// exercise — `predicate_support` is the same pipeline as an `emit` body, but
    /// the let-chain machinery only fires when a body actually declares one).
    #[test]
    fn predicate_support_single_column_resolves_to_that_column() {
        let support = predicate_support("score > 0.0").unwrap();
        assert_eq!(support, HashSet::from(["score".to_string()]));
    }

    /// A predicate over only literals parses cleanly and yields an EMPTY support
    /// set (no producer columns), which is distinct from a parse failure.
    #[test]
    fn predicate_support_constant_predicate_is_empty_not_none() {
        let support = predicate_support("1.0 > 0.0").expect("constant predicate parses");
        assert!(support.is_empty());
    }

    /// An unparseable predicate yields `None` — never inferred edges from garbage
    /// (the module-wide degrade-gracefully rule).
    #[test]
    fn predicate_support_parse_failure_is_none() {
        assert_eq!(predicate_support("region == == 'EU'"), None);
        assert_eq!(predicate_support("("), None);
    }

    /// A `let` introduces an intermediate name; the emit's resolved support is
    /// the let's *input columns*, never the let name itself.
    #[test]
    fn let_resolution_expands_to_input_columns() {
        let prog = program("let w = a + 1.0\nemit y = w * 2.0\n");
        let supports = emit_supports(&prog);
        assert_eq!(supports.len(), 1);
        let (target, support) = &supports[0];
        assert_eq!(target, "y");
        // Resolves to {a}; `w` must not leak into the support set.
        assert_eq!(support, &HashSet::from(["a".to_string()]));
    }

    /// Chained lets resolve transitively: y depends on the base columns of the
    /// whole chain, not on any intermediate let name.
    #[test]
    fn let_chain_resolves_transitively() {
        let prog = program("let u = a + b\nlet v = u * c\nemit y = v + 1.0\n");
        let supports = emit_supports(&prog);
        let (_, support) = &supports[0];
        assert_eq!(
            support,
            &HashSet::from(["a".to_string(), "b".to_string(), "c".to_string()])
        );
    }

    fn support_of<'a>(
        supports: &'a [(String, HashSet<String>)],
        target: &str,
    ) -> &'a HashSet<String> {
        supports
            .iter()
            .find(|(name, _)| name == target)
            .map(|(_, support)| support)
            .unwrap_or_else(|| panic!("emit `{target}` present"))
    }

    /// A field emitted inside `emit each` depends on the iterated source column,
    /// even though its own expression only references the loop binding (#150).
    #[test]
    fn emit_each_threads_source_support_to_body_emit() {
        let prog = program("emit each x in items {\n  emit y = x.v\n}\n");
        let supports = emit_supports(&prog);
        assert!(
            support_of(&supports, "y").contains("items"),
            "the iterated source column flows to the fanned-out field: {:?}",
            support_of(&supports, "y"),
        );
    }

    /// Nested fan-out accumulates: an inner body emit depends on EVERY enclosing
    /// `emit each` source (#150).
    #[test]
    fn nested_emit_each_unions_enclosing_sources() {
        let prog = program(
            "emit each x in items {\n  emit each w in extras {\n    emit z = w.q\n  }\n}\n",
        );
        let supports = emit_supports(&prog);
        let z = support_of(&supports, "z");
        assert!(
            z.contains("items") && z.contains("extras"),
            "both enclosing sources flow to the inner fanned-out field: {z:?}",
        );
    }

    /// An `emit each` source with empty column support (here a system-namespaced
    /// `$pipeline.items`, which `support_into` excludes) adds no spurious support
    /// — and therefore no spurious derive edge — to the fanned-out field (#150).
    #[test]
    fn emit_each_empty_support_source_adds_no_support() {
        let prog = program("emit each x in $pipeline.items {\n  emit y = x.v\n}\n");
        let supports = emit_supports(&prog);
        assert!(
            !support_of(&supports, "y").contains("items"),
            "an empty-support source contributes no column: {:?}",
            support_of(&supports, "y"),
        );
    }

    /// transform_output_fields: passthrough for unshadowed inputs (input
    /// order), then emitted targets (emit order). An emit that shadows an input
    /// column appears once, as Emitted. Both arithmetic emits infer `numeric`
    /// (#149); the carried input `x` has no inferred type here.
    #[test]
    fn output_fields_order_and_shadowing() {
        let prog = program("emit b = a * 2.0\nemit a = a + 1.0\n");
        let fields = transform_output_fields(&cols(&["a", "x"]), &prog);
        // `a` is shadowed by `emit a` → not a passthrough row; `x` rides through.
        assert_eq!(
            fields,
            vec![
                FieldRow {
                    name: "x".to_string(),
                    kind: FieldKind::PassThrough,
                    ty: None,
                    ..Default::default()
                },
                FieldRow {
                    name: "b".to_string(),
                    kind: FieldKind::Emitted,
                    ty: Some("numeric".to_string()),
                    ..Default::default()
                },
                FieldRow {
                    name: "a".to_string(),
                    kind: FieldKind::Emitted,
                    ty: Some("numeric".to_string()),
                    ..Default::default()
                },
            ]
        );
    }

    /// Aggregate rows are not transform rows: only group keys and aggregate
    /// emit targets appear, and copy-looking emits remain emitted fields.
    #[test]
    fn aggregate_output_fields_are_group_keys_then_emits() {
        let prog = program("emit total = sum(amount)\nemit copied_department = department\n");
        let fields = aggregate_output_fields(&cols(&["department", "region", "department"]), &prog);
        assert_eq!(
            fields,
            vec![
                FieldRow {
                    name: "department".to_string(),
                    kind: FieldKind::PassThrough,
                    ..Default::default()
                },
                FieldRow {
                    name: "region".to_string(),
                    kind: FieldKind::PassThrough,
                    ..Default::default()
                },
                FieldRow {
                    name: "total".to_string(),
                    kind: FieldKind::Emitted,
                    ..Default::default()
                },
                FieldRow {
                    name: "copied_department".to_string(),
                    kind: FieldKind::Emitted,
                    ..Default::default()
                },
            ]
        );
    }

    /// An emit that re-emits an input column unchanged (`emit a = a`, or a
    /// qualified `emit a = src.a`) is a passthrough copy — classified
    /// `PassThrough`, not `Emitted` — while a computed emit stays `Emitted`. A
    /// same-named `let` is NOT an input column, so it does not count as a copy.
    #[test]
    fn emit_identity_copy_is_passthrough_not_emitted() {
        // `a` re-emitted unchanged, `b` re-emitted via a qualified ref, `c`
        // computed. Inputs: a, b, x.
        let prog = program("emit a = a\nemit b = src.b\nemit c = a + 1.0\n");
        let copies = emit_copy_targets(&prog, &cols(&["a", "b", "x"]));
        assert_eq!(
            copies,
            HashSet::from(["a".to_string(), "b".to_string()]),
            "identity copies are detected; the computed `c` is not"
        );

        let fields = transform_output_fields(&cols(&["a", "b", "x"]), &prog);
        assert_eq!(
            fields,
            vec![
                // `x` rides through (unshadowed input).
                FieldRow {
                    name: "x".to_string(),
                    kind: FieldKind::PassThrough,
                    ty: None,
                    ..Default::default()
                },
                // `a`, `b` are identity copies → PassThrough (carried), not Emitted.
                FieldRow {
                    name: "a".to_string(),
                    kind: FieldKind::PassThrough,
                    ty: None,
                    ..Default::default()
                },
                FieldRow {
                    name: "b".to_string(),
                    kind: FieldKind::PassThrough,
                    ty: None,
                    ..Default::default()
                },
                // `c` is computed → Emitted (created/altered here); the
                // arithmetic emit infers `numeric` (#149).
                FieldRow {
                    name: "c".to_string(),
                    kind: FieldKind::Emitted,
                    ty: Some("numeric".to_string()),
                    ..Default::default()
                },
            ]
        );

        // A rename (`emit y = x`, single ref but different name) and a same-named
        // `let` are NOT copies — they produce genuinely new columns.
        let renamed = program("let a = x + 1.0\nemit y = x\nemit a = a\n");
        let copies = emit_copy_targets(&renamed, &cols(&["x"]));
        assert!(
            copies.is_empty(),
            "a rename and a let-shadowed name are not input-column copies"
        );
    }

    /// The 1-hop neighbourhood includes edges incident to the anchor in BOTH
    /// directions, but never a transitive (2-hop) edge.
    #[test]
    fn closure_walks_both_directions() {
        // Chain: 0.a -> 1.b -> 2.c, plus an unrelated edge 0.z -> 1.w.
        let edges = vec![
            FieldEdge {
                from_node: 0,
                from_field: "a".to_string(),
                to_node: 1,
                to_field: "b".to_string(),
                kind: FieldEdgeKind::Derive,
                ..Default::default()
            },
            FieldEdge {
                from_node: 1,
                from_field: "b".to_string(),
                to_node: 2,
                to_field: "c".to_string(),
                kind: FieldEdgeKind::Derive,
                ..Default::default()
            },
            FieldEdge {
                from_node: 0,
                from_field: "z".to_string(),
                to_node: 1,
                to_field: "w".to_string(),
                kind: FieldEdgeKind::Derive,
                ..Default::default()
            },
        ];
        // Hovering the middle anchor (1.b): edge 0 (its incoming) and edge 1 (its
        // outgoing) are both 1-hop incident; the unrelated edge 2 is not.
        let closure = lineage_closure(&edges, 1, "b");
        assert_eq!(closure, HashSet::from([0, 1]));

        // Hovering the chain head (0.a): only its outgoing edge 0 is incident —
        // the downstream b→c edge is 2 hops away and is NOT pulled in (1-hop).
        assert_eq!(lineage_closure(&edges, 0, "a"), HashSet::from([0]));

        // Hovering the unrelated target reaches only its own edge.
        assert_eq!(lineage_closure(&edges, 1, "w"), HashSet::from([2]));
    }

    /// #147: INDIRECT influence edges (Filter/JoinKey/Conditional/GroupBy) are
    /// kept VISIBLE by both reveal walks — selecting either endpoint of an
    /// INDIRECT edge returns that edge from `lineage_closure` (the 1-hop hover)
    /// AND from `field_lineage_full` (the transitive click). The walks are
    /// deliberately kind-agnostic (the canvas value/influence toggle is deferred to
    /// PR5's dual value/influence ribbon), so an INDIRECT edge is never permanently
    /// invisible. This fails if a future change filters either walk by `nature()`.
    #[test]
    fn indirect_edge_endpoints_are_revealed_by_closure_and_full_walk() {
        // A DIRECT carry 0.k -> 1.k, then an INDIRECT JoinKey 0.k -> 1.k (the
        // value carry and its influence overlay coexist), plus an INDIRECT Filter
        // that fans onward 1.k -> 2.kept.
        let edges = vec![
            FieldEdge {
                from_node: 0,
                from_field: "k".to_string(),
                to_node: 1,
                to_field: "k".to_string(),
                kind: FieldEdgeKind::Passthrough,
                ..Default::default()
            },
            FieldEdge {
                from_node: 0,
                from_field: "k".to_string(),
                to_node: 1,
                to_field: "k".to_string(),
                kind: FieldEdgeKind::JoinKey,
                ..Default::default()
            },
            FieldEdge {
                from_node: 1,
                from_field: "k".to_string(),
                to_node: 2,
                to_field: "kept".to_string(),
                kind: FieldEdgeKind::Filter,
                ..Default::default()
            },
        ];

        // Hovering the producer endpoint 0.k reveals BOTH incident edges at that
        // node — the DIRECT carry (0) AND the INDIRECT JoinKey (1).
        assert_eq!(
            lineage_closure(&edges, 0, "k"),
            HashSet::from([0, 1]),
            "the INDIRECT JoinKey edge must be revealed on hover, not hidden"
        );

        // Selecting 0.k transitively must surface the whole chain INCLUDING the
        // INDIRECT edges: the carry (0), the JoinKey (1), and the downstream
        // Filter (2) reached via the shared 1.k endpoint.
        assert_eq!(
            field_lineage_full(&edges, 0, "k"),
            HashSet::from([0, 1, 2]),
            "the transitive walk must keep INDIRECT edges visible end to end"
        );

        // Selecting the Filter's downstream endpoint 2.kept walks back up through
        // the INDIRECT edge to its producer — the INDIRECT edge is traversable in
        // both directions.
        assert!(
            field_lineage_full(&edges, 2, "kept").contains(&2),
            "selecting an INDIRECT edge's target must reveal that edge"
        );
    }

    /// A linear derive chain `0.f -> 1.f -> 2.f -> ... -> len.f`, with edge index
    /// `i` carrying node `i` to node `i+1`. Selecting node 0 walks the whole chain
    /// downstream; the edge crossed at depth `d` (0-indexed) is edge `d`.
    fn linear_chain(len: usize) -> Vec<FieldEdge> {
        (0..len)
            .map(|i| FieldEdge {
                from_node: i,
                from_field: "f".to_string(),
                to_node: i + 1,
                to_field: "f".to_string(),
                kind: FieldEdgeKind::Derive,
                ..Default::default()
            })
            .collect()
    }

    /// #123: the hop cap bounds the closure to exactly the edges within `n` hops of
    /// the anchor, per direction. On a long downstream chain selected at its head,
    /// `Some(n)` keeps exactly edges `0..n`; `Some(0)` is empty; `None` is the
    /// whole chain. The bounded set is computed at the helper level so it is unit-
    /// testable without the canvas.
    #[test]
    fn hop_cap_bounds_closure_to_exact_depth() {
        let edges = linear_chain(6); // 0.f -> 1.f -> ... -> 6.f (6 edges)

        // Zero hops reveals nothing — no edge is within zero steps of the anchor.
        assert!(field_lineage_full_capped(&edges, 0, "f", Some(0)).is_empty());

        // From the head, only downstream edges exist; `Some(n)` keeps edges 0..n.
        assert_eq!(
            field_lineage_full_capped(&edges, 0, "f", Some(1)),
            HashSet::from([0]),
            "1-hop keeps exactly the anchor's incident edge",
        );
        assert_eq!(
            field_lineage_full_capped(&edges, 0, "f", Some(3)),
            HashSet::from([0, 1, 2]),
            "3-hop keeps exactly the first three chain edges",
        );

        // A cap at or beyond the chain length equals the uncapped walk.
        let uncapped = field_lineage_full(&edges, 0, "f");
        assert_eq!(uncapped, HashSet::from([0, 1, 2, 3, 4, 5]));
        assert_eq!(
            field_lineage_full_capped(&edges, 0, "f", Some(6)),
            uncapped,
            "a cap == chain length matches the uncapped closure",
        );
        assert_eq!(
            field_lineage_full_capped(&edges, 0, "f", Some(99)),
            uncapped,
            "a cap beyond the chain length matches the uncapped closure",
        );
        assert_eq!(
            field_lineage_full_capped(&edges, 0, "f", None),
            uncapped,
            "None is the uncapped closure",
        );
    }

    /// #123: the cap counts hops PER DIRECTION. Selecting a mid-chain anchor with
    /// `Some(2)` reaches two edges upstream AND two downstream of the anchor, so a
    /// symmetric anchor yields a symmetric four-edge window.
    #[test]
    fn hop_cap_counts_each_direction_independently() {
        let edges = linear_chain(6); // edges 0..6 across nodes 0..7
        // Anchor node 3 sits between incoming edge 2 (2->3) and outgoing edge 3
        // (3->4). Two hops up reaches edges 2,1; two hops down reaches edges 3,4.
        assert_eq!(
            field_lineage_full_capped(&edges, 3, "f", Some(2)),
            HashSet::from([1, 2, 3, 4]),
            "a 2-hop cap window is two edges up and two edges down of the anchor",
        );
    }

    /// #123: raising the cap expands the bounded set MONOTONICALLY — each larger
    /// cap is a superset of the smaller — so "expand further" only ever reveals
    /// more, never reshuffles. Recomputed in a fresh loop each run; the assertion
    /// holds regardless of `HashSet` iteration order because it compares set
    /// membership, not ordering.
    #[test]
    fn hop_cap_expands_monotonically() {
        let edges = linear_chain(8);
        let mut prev = field_lineage_full_capped(&edges, 0, "f", Some(0));
        assert!(prev.is_empty());
        for cap in 1..=8 {
            let cur = field_lineage_full_capped(&edges, 0, "f", Some(cap));
            assert!(
                prev.is_subset(&cur),
                "cap {cap} must be a superset of cap {}: {prev:?} ⊄ {cur:?}",
                cap - 1,
            );
            assert_eq!(
                cur.len(),
                cap,
                "each added hop reveals exactly one more chain edge",
            );
            prev = cur;
        }
    }

    /// #123: the capped walk is DETERMINISTIC — identical input yields an identical
    /// set every time. Run repeatedly with a branchy graph (multiple producers and
    /// consumers per endpoint exercise `HashSet`-ordered iteration) to prove the
    /// breadth-first layering does not let map ordering leak into the result.
    #[test]
    fn hop_cap_is_deterministic_across_runs() {
        // A diamond that re-converges: 0.a feeds both 1.b and 1.c, which both feed
        // 2.d; plus a longer tail 2.d -> 3.e -> 4.g.
        let derive = |fnode: usize, ff: &str, tnode: usize, tf: &str| FieldEdge {
            from_node: fnode,
            from_field: ff.to_string(),
            to_node: tnode,
            to_field: tf.to_string(),
            kind: FieldEdgeKind::Derive,
            ..Default::default()
        };
        let edges = vec![
            derive(0, "a", 1, "b"),
            derive(0, "a", 1, "c"),
            derive(1, "b", 2, "d"),
            derive(1, "c", 2, "d"),
            derive(2, "d", 3, "e"),
            derive(3, "e", 4, "g"),
        ];
        let baseline = field_lineage_full_capped(&edges, 0, "a", Some(2));
        // Two hops down from 0.a: edges {0,1} (depth 1) then {2,3} (depth 2); the
        // 2.d -> 3.e edge is depth 3 and excluded.
        assert_eq!(baseline, HashSet::from([0, 1, 2, 3]));
        for _ in 0..64 {
            assert_eq!(
                field_lineage_full_capped(&edges, 0, "a", Some(2)),
                baseline,
                "the capped closure must be identical on every run",
            );
        }
    }

    /// #123: the Filter keep-set is exactly the anchor plus the endpoints of every
    /// participating edge, and an edge is safe to draw iff BOTH endpoints are kept
    /// — so Filter mode hides off-path clutter with no dangling partial paths AND
    /// retains every node on a connecting path between two kept nodes.
    #[test]
    fn filter_keep_set_has_no_dangling_paths() {
        // Two independent chains share node 1 as a hub:
        //   on-path:  0.a -> 1.a -> 2.a   (the anchor's lineage)
        //   off-path: 9.z -> 1.z          (touches the hub but not the anchor field)
        let derive = |fnode: usize, ff: &str, tnode: usize, tf: &str| FieldEdge {
            from_node: fnode,
            from_field: ff.to_string(),
            to_node: tnode,
            to_field: tf.to_string(),
            kind: FieldEdgeKind::Derive,
            ..Default::default()
        };
        let edges = vec![
            derive(0, "a", 1, "a"), // 0: on-path
            derive(1, "a", 2, "a"), // 1: on-path
            derive(9, "z", 1, "z"), // 2: off-path (different field on the hub node)
        ];

        // Select the chain head 0.a: the closure is the directed lineage of `a`.
        let closure = field_lineage_full(&edges, 0, "a");
        assert_eq!(
            closure,
            HashSet::from([0, 1]),
            "off-path edge 2 is excluded"
        );

        let participating: Vec<&FieldEdge> = closure.iter().map(|&i| &edges[i]).collect();
        let keep = lineage_keep_nodes(participating.iter().copied(), 0);

        // The keep-set is exactly the anchor's lineage nodes 0,1,2.
        assert_eq!(keep, HashSet::from([0, 1, 2]));
        // The off-path producer node 9 (only reachable via the unrelated `z` edge)
        // is excluded — Filter mode hides it.
        assert!(!keep.contains(&9), "off-path node must be filtered out");

        // No dangling edges: every participating edge has BOTH endpoints kept, so
        // the canvas's both-endpoints-kept draw predicate never leaves a half-edge.
        for edge in &participating {
            assert!(
                keep.contains(&edge.from_node) && keep.contains(&edge.to_node),
                "a kept edge has both endpoints in the keep-set: {edge:?}",
            );
        }

        // The off-path edge fails the both-endpoints predicate (node 9 absent), so
        // it would not be drawn — confirming the predicate suppresses it.
        let off_path = &edges[2];
        assert!(
            !(keep.contains(&off_path.from_node) && keep.contains(&off_path.to_node)),
            "an off-path edge is suppressed by the both-endpoints-kept predicate",
        );
    }

    /// #123: a node that lies on a connecting path BETWEEN two kept endpoints (a
    /// midpoint of the anchor's transitive lineage) is itself retained — the
    /// closure yields full up+down paths, so participating nodes form connected
    /// paths with no interior gap.
    #[test]
    fn filter_keep_set_retains_connecting_path_midpoint() {
        let edges = linear_chain(4); // 0.f -> 1.f -> 2.f -> 3.f -> 4.f
        // Select the MIDDLE node 2: its lineage spans the whole chain (up to 0,
        // down to 4). Every interior node 1,2,3 — including the midpoints between
        // the kept ends 0 and 4 — must be retained.
        let closure = field_lineage_full(&edges, 2, "f");
        let participating: Vec<&FieldEdge> = closure.iter().map(|&i| &edges[i]).collect();
        let keep = lineage_keep_nodes(participating.iter().copied(), 2);
        assert_eq!(
            keep,
            HashSet::from([0, 1, 2, 3, 4]),
            "every node on the connecting path between the kept ends is retained",
        );
    }

    /// #123: the anchor is always kept, even with NO participating edges (a leaf
    /// column on a single-node graph), so a Filter reveal never hides the very card
    /// the user selected.
    #[test]
    fn filter_keep_set_always_retains_lonely_anchor() {
        let keep = lineage_keep_nodes(std::iter::empty(), 7);
        assert_eq!(keep, HashSet::from([7]));
    }

    /// `group_endpoints_by_node` records BOTH sides of every supplied edge under
    /// its node, so a hover reveal can tint exactly the source/target field cells
    /// — the hovered field itself, its adjacent producers/consumers, and a
    /// self-loop endpoint (counted once) — while a field on an edge NOT supplied
    /// (e.g. one whose anchor failed to resolve, or an unrelated edge) is absent.
    #[test]
    fn field_endpoints_cover_both_sides_of_closure_edges() {
        let derive = |fnode: usize, ff: &str, tnode: usize, tf: &str| FieldEdge {
            from_node: fnode,
            from_field: ff.to_string(),
            to_node: tnode,
            to_field: tf.to_string(),
            kind: FieldEdgeKind::Derive,
            ..Default::default()
        };
        // Middle node 1 carries `status`, which also feeds a derive (`risk`) on
        // its OWN node — a self-loop on field `status`. Edge 3 is unrelated.
        let edges = vec![
            derive(0, "status", 1, "status"), // 0: into node 1
            derive(1, "status", 1, "risk"),   // 1: self-loop on node 1 feeding a derive
            derive(1, "status", 2, "status"), // 2: out of node 1
            derive(0, "age", 1, "risk"),      // 3: unrelated source field
        ];

        // Mirror the canvas path: take the REAL 1-hop closure of the hovered
        // field, then group the endpoints of exactly those (resolved) edges.
        let closure = lineage_closure(&edges, 1, "status");
        assert_eq!(closure, HashSet::from([0, 1, 2]));
        let grouped = group_endpoints_by_node(closure.iter().map(|&ei| &edges[ei]));

        // Each node maps to its de-duplicated, SORTED endpoint field names. Both
        // sides of every closure edge are present: node 0's `status` (producer),
        // node 1's `status` (hovered) + `risk` (the self-loop derive it feeds),
        // node 2's `status` (consumer).
        assert_eq!(
            grouped,
            HashMap::from([
                (0, vec!["status".to_string()]),
                (1, vec!["risk".to_string(), "status".to_string()]), // sorted
                (2, vec!["status".to_string()]),
            ]),
        );

        // Negative (standalone): the unrelated edge 3 (`0.age -> 1.risk`) is
        // outside the closure, so `age` never appears on ANY node.
        assert!(
            !grouped.values().flatten().any(|f| f == "age"),
            "a field only on a non-closure edge is never highlighted"
        );

        // The self-loop edge (1.status -> 1.risk) contributes node 1's `status`
        // once, not twice — the per-node accumulation de-duplicates.
        let self_loop_only = group_endpoints_by_node(std::iter::once(&edges[1]));
        assert_eq!(
            self_loop_only,
            HashMap::from([(1, vec!["risk".to_string(), "status".to_string()])]),
        );

        // No edges → no highlighted nodes (the not-hovered render path).
        assert!(group_endpoints_by_node(std::iter::empty()).is_empty());
    }

    /// The 1-hop neighbourhood INCLUDES passthrough carries (the FIX): a field
    /// reached only by an identity carry is no longer hidden.
    ///
    /// FIX C (#70 follow-up): the prior walk was transitive AND derive-only, so a
    /// 1:1 carry never appeared on hover. The model is
    /// `input{a} -> t1 emit b = a*2 -> t2 emit c = b + 1` with the realistic
    /// passthrough carries `a`/`b` ride through on; hovering a carried column now
    /// reveals its in/out carries, and hovering a derived column reveals exactly
    /// its immediate (1-hop) producers and consumers.
    #[test]
    fn closure_is_direct_one_hop_including_passthrough() {
        // Indices: 0=input, 1=t1, 2=t2.
        let derive = |fnode: usize, ff: &str, tnode: usize, tf: &str| FieldEdge {
            from_node: fnode,
            from_field: ff.to_string(),
            to_node: tnode,
            to_field: tf.to_string(),
            kind: FieldEdgeKind::Derive,
            ..Default::default()
        };
        let carry = |fnode: usize, ff: &str, tnode: usize, tf: &str| FieldEdge {
            from_node: fnode,
            from_field: ff.to_string(),
            to_node: tnode,
            to_field: tf.to_string(),
            kind: FieldEdgeKind::Passthrough,
            ..Default::default()
        };
        let edges = vec![
            derive(0, "a", 1, "b"), // 0: t1 computes b from a
            carry(0, "a", 1, "a"),  // 1: a rides through t1
            derive(1, "b", 2, "c"), // 2: t2 computes c from b
            carry(1, "a", 2, "a"),  // 3: a rides through t2
            carry(1, "b", 2, "b"),  // 4: b rides through t2
        ];

        // Hovering `c` on t2: only its single incoming derive edge (2) is 1-hop
        // incident. The upstream a→b derive (edge 0) is now 2 hops away and is
        // NOT pulled in — direct-neighbour scope, not transitive.
        assert_eq!(
            lineage_closure(&edges, 2, "c"),
            HashSet::from([2]),
            "c's 1-hop closure is exactly its incoming derive edge"
        );

        // Hovering the carried column `a` on t2: its incoming carry (edge 3) is
        // incident and IS revealed — passthrough is no longer excluded.
        assert_eq!(
            lineage_closure(&edges, 2, "a"),
            HashSet::from([3]),
            "a carried field reveals its incident passthrough carry"
        );
    }

    /// A hovered passthrough field surfaces its incoming AND outgoing passthrough
    /// carries PLUS any derive edge it feeds — all in a single 1-hop neighbourhood.
    ///
    /// Models the spec's `order_age.status` example: `status` rides through as a
    /// passthrough into the node, feeds a `fulfillment_risk` derive on that node,
    /// and carries on to the next node as a passthrough. Hovering `status` on the
    /// middle node must return its incoming carry, its outgoing carry, AND the
    /// derive it feeds — but not edges anchored on a different field.
    #[test]
    fn passthrough_field_returns_carries_and_fed_derive() {
        // Indices: 0=order_age (producer), 1=mid node, 2=next node.
        let derive = |fnode: usize, ff: &str, tnode: usize, tf: &str| FieldEdge {
            from_node: fnode,
            from_field: ff.to_string(),
            to_node: tnode,
            to_field: tf.to_string(),
            kind: FieldEdgeKind::Derive,
            ..Default::default()
        };
        let carry = |fnode: usize, ff: &str, tnode: usize, tf: &str| FieldEdge {
            from_node: fnode,
            from_field: ff.to_string(),
            to_node: tnode,
            to_field: tf.to_string(),
            kind: FieldEdgeKind::Passthrough,
            ..Default::default()
        };
        let edges = vec![
            carry(0, "status", 1, "status"), // 0: status rides INTO node 1
            derive(1, "status", 1, "fulfillment_risk"), // 1: status feeds a derive on node 1
            carry(1, "status", 2, "status"), // 2: status rides OUT of node 1
            derive(0, "age", 1, "fulfillment_risk"), // 3: unrelated input feeding the derive
        ];

        // Hovering `status` on node 1 returns: its incoming carry (0), the derive
        // it feeds (1), and its outgoing carry (2). Edge 3 is anchored on `age`,
        // not `status`, so it is excluded.
        assert_eq!(
            lineage_closure(&edges, 1, "status"),
            HashSet::from([0, 1, 2]),
            "a passthrough field surfaces its in/out carries plus the derive it feeds"
        );
    }

    /// `field_lineage_full` walks the column's COMPLETE directed lineage
    /// (transitive upstream ∪ downstream) — the click-to-select reveal (#75) —
    /// unlike the deliberately 1-hop `lineage_closure`; and it stays DIRECTED, so
    /// it does not flood into a sibling branch that merely shares an ancestor.
    #[test]
    fn field_lineage_full_is_transitive_directed_not_sibling_flood() {
        let carry = |fnode: usize, ff: &str, tnode: usize, tf: &str| FieldEdge {
            from_node: fnode,
            from_field: ff.to_string(),
            to_node: tnode,
            to_field: tf.to_string(),
            kind: FieldEdgeKind::Passthrough,
            ..Default::default()
        };
        let derive = |fnode: usize, ff: &str, tnode: usize, tf: &str| FieldEdge {
            from_node: fnode,
            from_field: ff.to_string(),
            to_node: tnode,
            to_field: tf.to_string(),
            kind: FieldEdgeKind::Derive,
            ..Default::default()
        };
        let edges = vec![
            carry(0, "a", 1, "a"),  // 0: a carried 0→1
            carry(1, "a", 2, "a"),  // 1: a carried 1→2 (a sibling downstream branch)
            derive(1, "a", 1, "b"), // 2: a feeds b on node 1
            carry(1, "b", 2, "b"),  // 3: b carried 1→2
            carry(0, "z", 1, "z"),  // 4: unrelated column
        ];

        // From the origin `a`: the downstream walk reaches the whole a/b subtree
        // (edges 0,1,2,3); there is no upstream. Transitive, not 1-hop.
        assert_eq!(
            field_lineage_full(&edges, 0, "a"),
            HashSet::from([0, 1, 2, 3])
        );
        // Contrast: the HOVER (1-hop) reveal on the same anchor is just its single
        // outgoing carry.
        assert_eq!(lineage_closure(&edges, 0, "a"), HashSet::from([0]));

        // From the downstream `b` on node 2: the upstream walk reaches b←b←a←a
        // (edges 3,2,0) but NOT the sibling branch a→2.a (edge 1): `2.a` is a
        // different descendant of `a`, not part of `2.b`'s lineage — a directed
        // walk, not an undirected connected-component flood.
        let from_b = field_lineage_full(&edges, 2, "b");
        assert_eq!(from_b, HashSet::from([0, 2, 3]));
        assert!(
            !from_b.contains(&1),
            "sibling branch a→2.a is not in 2.b's lineage"
        );
        assert!(
            !from_b.contains(&4),
            "the unrelated column z is never reached"
        );
    }

    /// `field_lineage_full` must TERMINATE on a cyclic edge set — the canvas
    /// renders pre-validation input, which can contain a column that carries back
    /// into an upstream node. The per-direction `seen` set bounds the walk; both
    /// edges of the cycle are still returned (a back-edge is recorded, just not
    /// re-traversed).
    #[test]
    fn field_lineage_full_terminates_on_cycle() {
        let carry = |fnode: usize, ff: &str, tnode: usize, tf: &str| FieldEdge {
            from_node: fnode,
            from_field: ff.to_string(),
            to_node: tnode,
            to_field: tf.to_string(),
            kind: FieldEdgeKind::Passthrough,
            ..Default::default()
        };
        // 0.a → 1.a → 0.a: a 2-edge cycle over the (node, field) endpoint graph.
        let edges = vec![carry(0, "a", 1, "a"), carry(1, "a", 0, "a")];
        // Terminates (no hang / stack overflow) and reaches both edges from either
        // anchor — downstream and upstream each close the loop in one extra hop.
        assert_eq!(field_lineage_full(&edges, 0, "a"), HashSet::from([0, 1]));
        assert_eq!(field_lineage_full(&edges, 1, "a"), HashSet::from([0, 1]));
    }

    /// `infer_emit_type` (#149) covers the common emit shapes and falls back to
    /// `Any` (the liberal Unknown) for everything else — never a wrong type.
    #[test]
    fn infer_emit_type_covers_common_shapes() {
        let builtins = BuiltinRegistry::new();
        let infer = |src: &str| {
            let prog = program(src);
            let emit_types =
                emit_target_types(&prog, &build_let_types(&prog, &builtins), &builtins);
            emit_types.get("y").cloned().expect("emit y present")
        };

        // Literals → their concrete type.
        assert_eq!(infer("emit y = 1\n"), Type::Int);
        assert_eq!(infer("emit y = 1.5\n"), Type::Float);
        assert_eq!(infer("emit y = \"hi\"\n"), Type::String);
        assert_eq!(infer("emit y = true\n"), Type::Bool);

        // Arithmetic with a numeric signal → the honest `Numeric`
        // over-approximation (the engine narrows to Int/Float once operand types
        // are known).
        assert_eq!(infer("emit y = a + 1\n"), Type::Numeric);
        // `*`/`-`/`/`/`%` have no non-numeric overload, so even two unknown
        // operands can only be a numeric op in a valid program.
        assert_eq!(infer("emit y = a * b\n"), Type::Numeric);

        // `+` is overloaded. Two known strings concatenate → String; two unknown
        // operands could be add *or* concat, so we abstain (Any) rather than
        // guess `numeric` — this is the conservatism guarantee (#149).
        assert_eq!(infer("emit y = \"a\" + \"b\"\n"), Type::String);
        assert_eq!(infer("emit y = a + b\n"), Type::Any);

        // Comparisons and logical ops → Bool.
        assert_eq!(infer("emit y = a > 3\n"), Type::Bool);
        assert_eq!(infer("emit y = a == b\n"), Type::Bool);
        assert_eq!(infer("emit y = a and b\n"), Type::Bool);
        assert_eq!(infer("emit y = not a\n"), Type::Bool);

        // String method → the builtin's declared return type.
        assert_eq!(infer("emit y = name.upper()\n"), Type::String);
        assert_eq!(infer("emit y = name.starts_with(\"x\")\n"), Type::Bool);

        // A bare input-column reference is Unknown in raw mode (no schema here).
        assert_eq!(infer("emit y = a\n"), Type::Any);
    }

    /// A known string `let` feeding `+` concatenates (#149): the overload is
    /// resolved by the operand's inferred type, not guessed as numeric.
    #[test]
    fn infer_add_resolves_string_concat_via_let_types() {
        let builtins = BuiltinRegistry::new();
        // `s` is a known string (method return), so `s + s` is concatenation.
        let prog = program("let s = name.upper()\nemit y = s + s\n");
        let let_types = build_let_types(&prog, &builtins);
        let emit_types = emit_target_types(&prog, &let_types, &builtins);
        assert_eq!(emit_types.get("y"), Some(&Type::String));
    }

    /// Type inference resolves `let`-chains transitively, the type analogue of
    /// `let_chain_resolves_transitively` (#149).
    #[test]
    fn infer_emit_type_resolves_let_chains() {
        let builtins = BuiltinRegistry::new();
        // `u` is numeric (arithmetic); `v` aliases `u`; `y` aliases `v` → numeric.
        let prog = program("let u = a + 1.0\nlet v = u\nemit y = v\n");
        let let_types = build_let_types(&prog, &builtins);
        assert_eq!(let_types.get("u"), Some(&Type::Numeric));
        assert_eq!(let_types.get("v"), Some(&Type::Numeric));
        let emit_types = emit_target_types(&prog, &let_types, &builtins);
        assert_eq!(emit_types.get("y"), Some(&Type::Numeric));

        // A string-typed let flows through to its emit.
        let prog = program("let s = name.upper()\nemit y = s\n");
        let let_types = build_let_types(&prog, &builtins);
        let emit_types = emit_target_types(&prog, &let_types, &builtins);
        assert_eq!(emit_types.get("y"), Some(&Type::String));
    }

    /// A cyclic let-support map must not loop forever; the visiting guard makes
    /// expansion total. (Declaration-ordered CXL can't produce this, but the
    /// canvas renders pre-validation input.)
    #[test]
    fn resolve_support_guards_cycles() {
        let mut let_support: HashMap<String, HashSet<String>> = HashMap::new();
        let_support.insert("p".to_string(), HashSet::from(["q".to_string()]));
        let_support.insert("q".to_string(), HashSet::from(["p".to_string()]));
        let raw = HashSet::from(["p".to_string()]);
        // Terminates and yields no genuine columns (both members are lets).
        let resolved = resolve_support(&raw, &let_support);
        assert!(resolved.is_empty());
    }

    /// `Precision::worst` keeps the LESS-precise tier (Unknown > Approximate >
    /// Exact) and is commutative. The fold uses it to roll a row's precision up
    /// from its incident edges, so getting the order wrong would silently report a
    /// degraded field as Exact — the exact failure #148 exists to prevent.
    #[test]
    fn precision_worst_keeps_least_precise_tier() {
        use Precision::{Approximate, Exact, Unknown};
        assert_eq!(Exact.worst(Approximate), Approximate);
        assert_eq!(Approximate.worst(Exact), Approximate);
        assert_eq!(Approximate.worst(Unknown), Unknown);
        assert_eq!(Unknown.worst(Approximate), Unknown);
        assert_eq!(Exact.worst(Unknown), Unknown);
        assert_eq!(Exact.worst(Exact), Exact);
        // Default is Exact so `FieldRow`/`FieldEdge` derive a clean zero value.
        assert_eq!(Precision::default(), Exact);
    }

    /// `precision_attr` feeds the `data-precision` HTML/CSS attribute, so every
    /// tier's slug must be a single whitespace-free token. Listing every variant by
    /// name (no wildcard) fails to compile if a tier is added without a slug
    /// decision — mirroring `edge_kind_attr_is_always_slug_safe`.
    #[test]
    fn precision_attr_is_always_slug_safe() {
        for precision in [Precision::Exact, Precision::Approximate, Precision::Unknown] {
            let attr = precision.precision_attr();
            assert!(
                !attr.is_empty(),
                "{precision:?} must have a non-empty data-precision slug"
            );
            assert!(
                !attr.chars().any(char::is_whitespace),
                "{precision:?} data-precision slug must contain no whitespace, got {attr:?}"
            );
        }
    }

    /// A straight-line emit is NOT fanned; only emits inside an `emit each` body
    /// are. The fan-out membership is what drops a derive edge to Approximate
    /// (#148), so this locks the boundary the classifier reads.
    #[test]
    fn emit_each_fanned_targets_flags_only_fan_out_bodies() {
        let prog = program("emit y = a + 1.0\nemit each x in items {\n  emit z = x.v\n}\n");
        let fanned = emit_each_fanned_targets(&prog);
        assert!(fanned.contains("z"), "the fanned body emit `z` is flagged");
        assert!(
            !fanned.contains("y"),
            "the straight-line emit `y` is NOT flagged: {fanned:?}"
        );
    }

    /// An `emit each` whose source resolves to NO column (a system-namespaced
    /// source) still flags its body emits as fanned (#148): the fan-out itself is
    /// the source of imprecision, independent of the iterated source's column
    /// support — distinct from `emit_each_empty_support_source_adds_no_support`,
    /// which is about derive *edges*, not the precision flag.
    #[test]
    fn emit_each_fanned_targets_flags_even_empty_support_source() {
        let prog = program("emit each x in $pipeline.items {\n  emit y = x.v\n}\n");
        let fanned = emit_each_fanned_targets(&prog);
        assert!(
            fanned.contains("y"),
            "a fanned emit is flagged regardless of its source's column support: {fanned:?}"
        );
    }

    /// #148 S1 (documented conservative behaviour): the fanned set is keyed by field
    /// NAME, so a field emitted BOTH by a straight-line emit AND inside a fan-out
    /// body on the same node is reported fanned. This degrades the straight-line
    /// derive to Approximate too — over-approximation in the SAFE direction (the
    /// field's value is partly per-element, so we never claim Exact). Locked here so
    /// a future change to statement-scoped degradation is a deliberate, tested move.
    #[test]
    fn emit_each_fanned_targets_is_conservative_per_field() {
        // `y` is emitted straight-line first, then re-emitted inside an `emit each`.
        let prog = program("emit y = a + 1.0\nemit each x in items {\n  emit y = x.v\n}\n");
        let fanned = emit_each_fanned_targets(&prog);
        assert!(
            fanned.contains("y"),
            "a field emitted both straight-line and fanned is conservatively flagged \
             fanned (per-field, not per-statement): {fanned:?}"
        );
    }

    /// The `FieldEdge` constructors classify precision deterministically from the
    /// edge kind and construction context (#148): a carry is Exact, a non-fanned
    /// derive is Exact, a fanned derive is Approximate, an INDIRECT influence is
    /// Approximate, and a conservative CXL-less fan-in is Approximate. This is the
    /// single classifier both lineage paths funnel through, so it is the place to
    /// lock the mapping.
    #[test]
    fn field_edge_constructors_classify_precision() {
        let carry = FieldEdge::carry(0, "k".into(), 1, "k".into(), FieldEdgeKind::Passthrough);
        assert_eq!(carry.precision, Precision::Exact);
        let access = FieldEdge::carry(0, "k".into(), 1, "k".into(), FieldEdgeKind::Access);
        assert_eq!(access.precision, Precision::Exact);

        let derive_clean = FieldEdge::derive(0, "a".into(), 1, "y".into(), false);
        assert_eq!(derive_clean.precision, Precision::Exact);
        assert_eq!(derive_clean.kind, FieldEdgeKind::Derive);
        let derive_fanned = FieldEdge::derive(0, "a".into(), 1, "y".into(), true);
        assert_eq!(derive_fanned.precision, Precision::Approximate);

        for kind in [
            FieldEdgeKind::Filter,
            FieldEdgeKind::GroupBy,
            FieldEdgeKind::JoinKey,
            FieldEdgeKind::Conditional,
        ] {
            let edge = FieldEdge::influence(0, "k".into(), 1, "k".into(), kind);
            assert_eq!(
                edge.precision,
                Precision::Approximate,
                "{kind:?} influence edge must be Approximate"
            );
        }

        let fan_in = FieldEdge::conservative_fan_in(0, "k".into(), 1, "k".into());
        assert_eq!(fan_in.precision, Precision::Approximate);
        assert_eq!(fan_in.kind, FieldEdgeKind::Passthrough);
    }

    /// Precision is EXCLUDED from `FieldEdge` identity (#148): two edges with the
    /// same 5-tuple but different precision/reason compare EQUAL, so `==` and the
    /// dedup key stay in lock-step. This guards the hand-written `PartialEq` against
    /// a future accidental `#[derive(PartialEq)]` that would split one edge in two.
    #[test]
    fn field_edge_equality_ignores_precision() {
        let exact = FieldEdge::derive(0, "a".into(), 1, "y".into(), false);
        let approx = FieldEdge::derive(0, "a".into(), 1, "y".into(), true);
        assert_ne!(exact.precision, approx.precision);
        assert_eq!(
            exact, approx,
            "edges with equal identity must compare equal regardless of precision"
        );
        // A differing identity field still breaks equality.
        let other = FieldEdge::derive(0, "a".into(), 1, "z".into(), false);
        assert_ne!(exact, other);
    }
}
