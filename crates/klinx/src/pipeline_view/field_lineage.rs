//! Pure field-level lineage analysis for composition canvases.
//!
//! This module is deliberately free of Dioxus: it parses CXL, derives each
//! body node's output field set, and computes the per-field edges that connect
//! a producer column to the consumer column it (partly) derives. The canvas
//! layer renders the results; all of the lineage *logic* lives here so it can
//! be unit-tested headlessly and reasoned about in isolation.
//!
//! Scope is Phase 1: compositions, transform-precise emit/passthrough rules,
//! and CXL `let`-chain resolution. Per-node-type precision for
//! aggregate/route/merge/combine is Phase 2 (#67) — those types are kept
//! "correct-ish" (passthrough + best-effort emit) and never panic.

use std::collections::{HashMap, HashSet};

use cxl::ast::{Program, Statement};
use cxl::parser::Parser;

/// How a field on a node's output record came to exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    /// An input/source column declared by a schema (input port or Source).
    Declared,
    /// A column written by an `emit name = expr` statement.
    Emitted,
    /// An input column carried through unchanged (not shadowed by an emit).
    PassThrough,
}

/// One row in a node's output record: a field name plus how it arose.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldRow {
    pub name: String,
    pub kind: FieldKind,
}

/// A field-level lineage edge: `to_node.to_field` is (partly) derived from
/// `from_node.from_field`.
///
/// `passthrough` distinguishes an identity carry (`col` → same `col`, the
/// value is unchanged) from a derivation (`col` participates in an expression
/// that produces a differently-named output column). The canvas renders the
/// two differently so a reader can tell a rename/compute from a carry.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldEdge {
    pub from_node: usize,
    pub from_field: String,
    pub to_node: usize,
    pub to_field: String,
    pub passthrough: bool,
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

    let mut rows: Vec<FieldRow> = Vec::new();
    for col in input_cols {
        if !emitted_set.contains(col.as_str()) {
            rows.push(FieldRow {
                name: col.clone(),
                kind: FieldKind::PassThrough,
            });
        }
    }
    for name in emitted {
        rows.push(FieldRow {
            name,
            kind: FieldKind::Emitted,
        });
    }
    rows
}

/// Output field rows for a node with no parseable CXL (or a non-emitting
/// type): every input column carried through unchanged.
///
/// Used both for nodes whose CXL failed to parse (fields still render, edges
/// skipped) and for Phase-1 best-effort handling of types without precise
/// emit rules.
pub fn passthrough_output_fields(input_cols: &[String]) -> Vec<FieldRow> {
    input_cols
        .iter()
        .map(|col| FieldRow {
            name: col.clone(),
            kind: FieldKind::PassThrough,
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
                    kind: FieldKind::PassThrough
                },
                FieldRow {
                    name: "b".to_string(),
                    kind: FieldKind::Emitted
                },
                FieldRow {
                    name: "a".to_string(),
                    kind: FieldKind::Emitted
                },
            ]
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
                passthrough: false,
            },
            FieldEdge {
                from_node: 1,
                from_field: "b".to_string(),
                to_node: 2,
                to_field: "c".to_string(),
                passthrough: false,
            },
            FieldEdge {
                from_node: 0,
                from_field: "z".to_string(),
                to_node: 1,
                to_field: "w".to_string(),
                passthrough: false,
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
            passthrough: false,
        };
        let carry = |fnode: usize, ff: &str, tnode: usize, tf: &str| FieldEdge {
            from_node: fnode,
            from_field: ff.to_string(),
            to_node: tnode,
            to_field: tf.to_string(),
            passthrough: true,
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
            passthrough: false,
        };
        let carry = |fnode: usize, ff: &str, tnode: usize, tf: &str| FieldEdge {
            from_node: fnode,
            from_field: ff.to_string(),
            to_node: tnode,
            to_field: tf.to_string(),
            passthrough: true,
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
