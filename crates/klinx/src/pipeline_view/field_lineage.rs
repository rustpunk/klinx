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

use cxl::ast::{Expr, Program, Statement};
use cxl::parser::Parser;

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
    /// that carry a typed producer column through unchanged. `None` for
    /// [`FieldKind::Emitted`] columns — typing an `emit` expression needs the
    /// engine typechecker (Phase 2b / #68) — and when the producer column has no
    /// known type.
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
    /// Whether this field participates in the post-Aggregate failure grain.
    ///
    /// Aggregate `group_by` keys become the grouped-record correlation grain:
    /// downstream failures are correlated by the aggregate group before the
    /// engine expands back to contributing source rows. This is not the same as
    /// a source-declared `correlation_key`, so it is tracked separately while
    /// still propagating through unchanged passthrough rows.
    pub is_aggregate_grain: bool,
}

/// How a [`FieldEdge`] relates its two endpoints — the three relationship
/// types the canvas colour-codes (#72).
///
/// A 3-way widening of the original `passthrough: bool` split: an identity
/// carry (`c → c`, value unchanged) is now sub-divided by whether the carried
/// column *also* feeds a computed/renamed output. The renderer maps each
/// variant to a distinct stroke colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldEdgeKind {
    /// Identity carry (`c → c`) whose column is read by no derive — it rides
    /// through the node untouched and unreferenced.
    Passthrough,
    /// Identity carry (`c → c`) whose column ALSO appears in some computed or
    /// renamed emit's support on the consumer: carried *and* accessed. The
    /// value still passes through unchanged, but the column is not inert here.
    Access,
    /// An input column feeding an `emit`-produced output field (computed or
    /// renamed), e.g. `line_total → value_tier`. The output value is (re)made
    /// from this input, not carried.
    Derive,
}

/// A field-level lineage edge: `to_node.to_field` is (partly) derived from
/// `from_node.from_field`.
///
/// `kind` ([`FieldEdgeKind`]) distinguishes an identity carry — pure
/// (`Passthrough`) or also-accessed (`Access`) — from a derivation (`Derive`).
/// The canvas renders the three differently so a reader can tell a
/// rename/compute from a carry, and a referenced carry from an inert one.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldEdge {
    pub from_node: usize,
    pub from_field: String,
    pub to_node: usize,
    pub to_field: String,
    pub kind: FieldEdgeKind,
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

    // Datatypes are filled in by the caller (`compute_field_lineage`): a
    // PassThrough row inherits its producer column's type; an Emitted row's type
    // would need the engine typechecker (Phase 2b), so it stays `None` here.
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
        rows.push(FieldRow {
            name,
            kind,
            ty: None,
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
                is_aggregate_grain: true,
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
pub fn emit_supports(program: &Program) -> Vec<(String, HashSet<String>)> {
    let let_support = build_let_support(program);
    let mut out: Vec<(String, HashSet<String>)> = Vec::new();
    // `for_each_field_emit` descends into `Statement::EmitEach.body`, so each
    // emitted field *inside* an `emit each` block is reported here. It does NOT
    // walk the `EmitEach.source` expression, so a fanned-out field's dependence
    // on the iterated source column is not yet captured.
    // Phase 2 (#67): EmitEach.source binding lineage not yet derived.
    cxl::ast::for_each_field_emit(&program.statements, &mut |name, expr| {
        let mut raw = HashSet::new();
        expr.support_into(&mut raw);
        let resolved = resolve_support(&raw, &let_support);
        out.push((name.to_string(), resolved));
    });
    out
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
    let mut result: HashSet<usize> = HashSet::new();

    // Upstream (provenance): follow edges INTO the current endpoint, back to
    // origins. Downstream (impact): follow edges OUT to sinks. Each direction is
    // its own breadth-first walk over the (node, field) endpoint graph, with its
    // own visited set; both deposit edge indices into the shared `result`.
    for forward in [false, true] {
        let mut seen: HashSet<(usize, String)> = HashSet::new();
        let start = (node, field.to_string());
        seen.insert(start.clone());
        let mut frontier = vec![start];
        while let Some((n, f)) = frontier.pop() {
            for (i, e) in edges.iter().enumerate() {
                // Forward walk steps producer→consumer (match the `from` side and
                // hop to `to`); backward walk steps consumer→producer.
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
                        frontier.push(other);
                    }
                }
            }
        }
    }
    result
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

    /// transform_output_fields: passthrough for unshadowed inputs (input
    /// order), then emitted targets (emit order). An emit that shadows an input
    /// column appears once, as Emitted.
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
                    ty: None,
                    ..Default::default()
                },
                FieldRow {
                    name: "a".to_string(),
                    kind: FieldKind::Emitted,
                    ty: None,
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
                    is_aggregate_grain: true,
                    ..Default::default()
                },
                FieldRow {
                    name: "region".to_string(),
                    kind: FieldKind::PassThrough,
                    is_aggregate_grain: true,
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
                // `c` is computed → Emitted (created/altered here).
                FieldRow {
                    name: "c".to_string(),
                    kind: FieldKind::Emitted,
                    ty: None,
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
            },
            FieldEdge {
                from_node: 1,
                from_field: "b".to_string(),
                to_node: 2,
                to_field: "c".to_string(),
                kind: FieldEdgeKind::Derive,
            },
            FieldEdge {
                from_node: 0,
                from_field: "z".to_string(),
                to_node: 1,
                to_field: "w".to_string(),
                kind: FieldEdgeKind::Derive,
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
        };
        let carry = |fnode: usize, ff: &str, tnode: usize, tf: &str| FieldEdge {
            from_node: fnode,
            from_field: ff.to_string(),
            to_node: tnode,
            to_field: tf.to_string(),
            kind: FieldEdgeKind::Passthrough,
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
        };
        let carry = |fnode: usize, ff: &str, tnode: usize, tf: &str| FieldEdge {
            from_node: fnode,
            from_field: ff.to_string(),
            to_node: tnode,
            to_field: tf.to_string(),
            kind: FieldEdgeKind::Passthrough,
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
        };
        let derive = |fnode: usize, ff: &str, tnode: usize, tf: &str| FieldEdge {
            from_node: fnode,
            from_field: ff.to_string(),
            to_node: tnode,
            to_field: tf.to_string(),
            kind: FieldEdgeKind::Derive,
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
        };
        // 0.a → 1.a → 0.a: a 2-edge cycle over the (node, field) endpoint graph.
        let edges = vec![carry(0, "a", 1, "a"), carry(1, "a", 0, "a")];
        // Terminates (no hang / stack overflow) and reaches both edges from either
        // anchor — downstream and upstream each close the loop in one extra hop.
        assert_eq!(field_lineage_full(&edges, 0, "a"), HashSet::from([0, 1]));
        assert_eq!(field_lineage_full(&edges, 1, "a"), HashSet::from([0, 1]));
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
}
