//! Node-preserving YAML sync for inspector edits.
//!
//! # Why this module exists
//!
//! The engine marks `PipelineConfig.nodes` `#[serde(skip_serializing)]`
//! (`clinker-core` `config/mod.rs`): the engine treats the source YAML as
//! authoritative for nodes, each node carrying a [`Spanned`] byte span into
//! that source. So a plain `clinker_core::yaml::to_string(config)` emits
//! `pipeline:` + `error_handling:` + `_notes` but **drops the entire `nodes:`
//! block**. Persisting that to disk, or pushing it back into the editor
//! buffer, silently destroys every node (klinx issue #29, P1 data-loss).
//!
//! # The fix: treat `yaml_text` as the authoritative document
//!
//! An inspector edit mutates `config` in memory (e.g. `set_stage_notes`). To
//! reflect that into the editor buffer without losing nodes, comments, or
//! formatting, [`patch_yaml_preserving_nodes`] **surgically patches the
//! current `yaml_text`** rather than regenerating it from `config`:
//!
//! 1. Re-parse the current `yaml_text` to recover the OLD node set (and the
//!    per-node source byte offsets via [`Spanned::referenced`]).
//! 2. Delimit each node's source region from the node start-offsets (the
//!    start offset is reliable; serde-saphyr reports a zero-length span, so
//!    region *ends* are derived from the next node's start and the end of the
//!    `nodes:` block).
//! 3. For each node, compare the OLD serialization against the edited one. An
//!    unchanged node keeps its source region **verbatim** (comments and
//!    formatting intact); only a changed node is re-serialized in place.
//! 4. Splice the rebuilt `nodes:` block back into the untouched head/tail.
//!
//! # Why a node is re-serialized rather than patched field-by-field
//!
//! `PipelineNode` itself derives `Serialize`; only the parent `nodes` *field*
//! is skipped. But individual fields inside a node (e.g. the `_notes` map)
//! lose their YAML span through the tagged-enum / buffered-dispatch
//! deserialization path, so there is no reliable byte range to patch a single
//! field. Re-serializing the whole changed node is the smallest safe unit. The
//! cost is borne only by the node the user is actively editing; every other
//! node and all comments survive byte-for-byte.
//!
//! # Fallback — never drop nodes
//!
//! When the current text cannot be parsed, the node counts diverge (an
//! add/remove rather than an in-place edit), or the non-node sections changed,
//! [`serialize_yaml_full`] rebuilds the document from `config` AND emits a real
//! `nodes:` block by serializing each `node.value`. It loses comments but is
//! never node-less — the data-loss invariant holds on every path.
//!
//! [`Spanned`]: clinker_core::yaml::Spanned
//! [`Spanned::referenced`]: clinker_core::yaml::Spanned

use clinker_core::config::{PipelineConfig, PipelineNode, parse_config};

/// Reflect an inspector edit of `config` into the authoritative `current_yaml`
/// document, preserving every unchanged node's text and the user's comments.
///
/// This is the preferred path for the inspector → YAML serialize effect. It
/// keeps `current_yaml` as the source of truth and patches only the node(s)
/// the edit actually changed. Falls back to [`serialize_yaml_full`] (which
/// rebuilds a full document with a real `nodes:` block) whenever surgical
/// patching is not provably safe — so the result is **never** node-less.
pub fn patch_yaml_preserving_nodes(current_yaml: &str, config: &PipelineConfig) -> String {
    match try_patch(current_yaml, config) {
        Some(patched) => patched,
        None => serialize_yaml_full(config),
    }
}

/// Attempt the surgical patch. Returns `None` (signalling the caller to fall
/// back to a full rebuild) when patching cannot be done safely.
fn try_patch(current_yaml: &str, config: &PipelineConfig) -> Option<String> {
    // Re-parse the authoritative text to recover the OLD nodes + their source
    // spans. A non-parsing buffer has no spans to patch against.
    let old = parse_config(current_yaml).ok()?;

    // In-place inspector edits never add or remove nodes; a count change means
    // we are not in the edit pattern this patcher handles. Fall back so we
    // never mis-align regions (and never drop a node).
    if old.nodes.len() != config.nodes.len() {
        return None;
    }

    // The non-node sections (`pipeline:`, `error_handling:`, top-level
    // `_notes:`) serialize losslessly through the engine, so we compare the
    // node-less serializations: if they differ, a non-node section changed and
    // a verbatim head/tail splice would drop that change. Fall back to a full
    // rebuild in that case (rare — the inspector only edits per-node notes
    // today). Equal node-less serializations ⇒ only nodes changed ⇒ the
    // head/tail can be kept byte-identical.
    let old_meta = clinker_core::yaml::to_string(&old).ok()?;
    let new_meta = clinker_core::yaml::to_string(config).ok()?;
    if old_meta != new_meta {
        return None;
    }

    // Recover each OLD node's source byte offset. serde-saphyr reports a
    // zero-length span whose offset points at the node's first key (the `type:`
    // line); we widen each to its containing line to capture the `- ` marker.
    let mut region_starts = Vec::with_capacity(old.nodes.len());
    for node in &old.nodes {
        let off = node.referenced.span().byte_offset()? as usize;
        if off > current_yaml.len() {
            return None;
        }
        region_starts.push(line_start(current_yaml, off));
    }

    // Node order must match the document order for region delimiting to be
    // sound. Inspector edits never reorder, but verify defensively.
    if region_starts.windows(2).any(|w| w[0] >= w[1]) {
        return None;
    }

    let block_start = *region_starts.first()?;
    let block_end = find_nodes_block_end(current_yaml, *region_starts.last()?);

    let mut out = String::with_capacity(current_yaml.len() + 64);
    // Head: everything up to and including the `nodes:` line and any leading
    // comments — kept verbatim.
    out.push_str(&current_yaml[..block_start]);

    for i in 0..old.nodes.len() {
        let rs = region_starts[i];
        let re = if i + 1 < region_starts.len() {
            region_starts[i + 1]
        } else {
            block_end
        };

        let old_ser = clinker_core::yaml::to_string(&old.nodes[i].value).ok()?;
        let new_ser = clinker_core::yaml::to_string(&config.nodes[i].value).ok()?;
        if old_ser == new_ser {
            // Unchanged node: keep its source region verbatim (comments,
            // inline formatting, blank lines, and any trailing inter-node
            // comment all survive).
            out.push_str(&current_yaml[rs..re]);
        } else {
            // Changed node: re-serialize just this one as a list item at the
            // same indent. Its own inline comments on data lines normalize away
            // — acceptable, it is the node being edited.
            //
            // A node's source region `[rs..re)` runs up to the next node's
            // start (or the block end), so it also absorbs any blank lines and
            // standalone comment lines that sit *between* this node's body and
            // the next node (an inter-node comment is visually "before the next
            // node" but byte-wise "after this node"). Those belong to the user,
            // not to this node's serialization — re-serializing the whole region
            // would silently delete them (issue #29 is a data-loss fix; dropping
            // a comment is the same class of regression). So split off the
            // trailing run of blank/comment lines and keep it verbatim; only the
            // node's own data lines are replaced.
            let gutter = node_trailing_gutter(current_yaml, rs, re);
            out.push_str(&serialize_node_as_list_item(
                &config.nodes[i].value,
                node_base_indent(current_yaml, rs),
            ));
            out.push_str(&current_yaml[gutter..re]);
        }
    }

    // Tail: everything after the `nodes:` block (sibling top-level keys such as
    // `error_handling:` / top-level `_notes:`, plus their comments) verbatim.
    out.push_str(&current_yaml[block_end..]);

    Some(out)
}

/// Serialize a `PipelineConfig` to YAML **including** a real `nodes:` block.
///
/// The engine's `to_string` omits `nodes` (`#[serde(skip_serializing)]`), so
/// this stitches a klinx-assembled `nodes:` block into the engine output. Used
/// as the fallback when surgical text-patching is not available (no
/// authoritative source text, unparsable buffer, structural node add/remove).
/// Loses comments and emits each node's defaulted fields, but is **never**
/// node-less — upholding the issue-#29 data-loss invariant on every path.
pub fn serialize_yaml_full(config: &PipelineConfig) -> String {
    let meta = match clinker_core::yaml::to_string(config) {
        Ok(s) => s,
        Err(e) => return format!("# Serialization error: {e}\n"),
    };

    // `nodes:` is a required key on `PipelineConfig` — the engine rejects a
    // document that omits it (`SerdeMissingField: nodes`). An empty node set
    // must therefore still emit an explicit `nodes: []` so the output re-parses;
    // returning the node-less `meta` would produce YAML the engine refuses to
    // load, breaking the round-trip on this fallback path.
    let nodes_block = if config.nodes.is_empty() {
        String::from("nodes: []\n")
    } else {
        let mut block = String::from("nodes:\n");
        for node in &config.nodes {
            block.push_str(&serialize_node_as_list_item(&node.value, 0));
        }
        block
    };

    // Insert the `nodes:` block directly after the `pipeline:` mapping so the
    // document reads in the canonical order (pipeline → nodes → error_handling
    // → _notes). If the `pipeline:` block can't be located, append the nodes
    // block — correctness (nodes present) over cosmetic ordering.
    match insertion_point_after_pipeline(&meta) {
        Some(idx) => {
            let mut out = String::with_capacity(meta.len() + nodes_block.len());
            out.push_str(&meta[..idx]);
            out.push_str(&nodes_block);
            out.push_str(&meta[idx..]);
            out
        }
        None => {
            let mut out = meta;
            out.push_str(&nodes_block);
            out
        }
    }
}

/// Re-serialize one `PipelineNode` and indent it as a `nodes:` list item at
/// `base_indent` (the column of the `- ` marker). `PipelineNode` is
/// `Serialize` (`#[serde(tag = "type")]`), so this yields a `type: …` mapping
/// that re-parses into the same node.
fn serialize_node_as_list_item(node: &PipelineNode, base_indent: usize) -> String {
    let body = match clinker_core::yaml::to_string(node) {
        Ok(s) => s,
        Err(e) => format!("# node serialization error: {e}\n"),
    };
    let pad = " ".repeat(base_indent);
    let mut out = String::with_capacity(body.len() + body.lines().count() * (base_indent + 2));
    for (i, line) in body.lines().enumerate() {
        out.push_str(&pad);
        // First line gets the `- ` sequence marker; the rest align two columns
        // deeper so they sit under the marker as the same map entry.
        out.push_str(if i == 0 { "- " } else { "  " });
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Byte offset of the start of the line containing byte `off`.
fn line_start(text: &str, off: usize) -> usize {
    text[..off].rfind('\n').map_or(0, |i| i + 1)
}

/// Indentation (column of the first non-space byte) of the line beginning at
/// `line_start_off`. This is the column of the node's `- ` marker.
fn node_base_indent(text: &str, line_start_off: usize) -> usize {
    let line_end = text[line_start_off..]
        .find('\n')
        .map(|p| line_start_off + p)
        .unwrap_or(text.len());
    let line = &text[line_start_off..line_end];
    line.len() - line.trim_start().len()
}

/// Byte offset within node region `[rs..re)` at which the trailing run of
/// blank and comment-only lines begins — i.e. the boundary between the node's
/// own data lines and the inter-node gutter (blank lines + standalone comments)
/// that precede the next node.
///
/// Walks the region's lines and tracks the offset just past the last
/// data-bearing line; lines whose trimmed content is empty or starts with `#`
/// after that point form the trailing gutter. The node's body is `[rs..gutter)`;
/// `[gutter..re)` is preserved verbatim when the node is re-serialized so the
/// user's inter-node comments and spacing survive. If the whole region is data
/// (no trailing gutter), returns `re`.
fn node_trailing_gutter(text: &str, rs: usize, re: usize) -> usize {
    let region = &text[rs..re];
    // Walk lines, remembering the start offset of the last data-bearing line's
    // *successor* — that is where the trailing gutter begins.
    let mut gutter = rs;
    let mut at = rs;
    for line in region.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let is_blank_or_comment = trimmed.is_empty() || trimmed.starts_with('#');
        at += line.len();
        if !is_blank_or_comment {
            // Data line: the gutter (if any) starts after this line.
            gutter = at;
        }
    }
    gutter
}

/// From a byte offset inside the `nodes:` block, find where the block ends.
///
/// Scans forward for the first subsequent line that is non-blank, non-comment,
/// and indented at column 0 — a sibling top-level key (`error_handling:`,
/// top-level `_notes:`, …). Returns that line's start offset, or `text.len()`
/// at EOF. The returned offset is the exclusive end of the last node's region.
fn find_nodes_block_end(text: &str, from: usize) -> usize {
    let mut idx = from;
    while idx < text.len() {
        let next = match text[idx..].find('\n') {
            Some(p) => idx + p + 1,
            None => return text.len(),
        };
        if next >= text.len() {
            return text.len();
        }
        let line_end = text[next..]
            .find('\n')
            .map(|p| next + p)
            .unwrap_or(text.len());
        let line = &text[next..line_end];
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if !trimmed.is_empty() && !trimmed.starts_with('#') && indent == 0 {
            return next;
        }
        idx = next;
    }
    text.len()
}

/// Byte offset at which to insert a `nodes:` block within an engine
/// serialization that lacks one: immediately after the `pipeline:` mapping
/// (i.e. before the first subsequent column-0 key, or EOF). Returns `None` if
/// no `pipeline:` key is present.
fn insertion_point_after_pipeline(meta: &str) -> Option<usize> {
    let mut idx = 0usize;
    let mut seen_pipeline = false;
    for line in meta.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let indent = line.len() - line.trim_start().len();
        let is_top_key = indent == 0 && !trimmed.is_empty() && !trimmed.starts_with('#');
        if is_top_key {
            if !seen_pipeline {
                if trimmed.starts_with("pipeline:") {
                    seen_pipeline = true;
                }
            } else {
                // First top-level key after `pipeline:` — insert before it.
                return Some(idx);
            }
        }
        idx += line.len();
    }
    if seen_pipeline {
        Some(meta.len())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A three-node pipeline with comments, an inline comment on a node, an
    /// inter-node comment, a leading comment, trailing `error_handling:`, and
    /// a top-level `_notes:` block. Exercises every preservation path.
    const FIXTURE: &str = r#"pipeline:
  name: demo

# a leading comment
nodes:
  - type: source
    name: raw          # inline comment on raw
    config:
      name: raw
      type: csv
      path: data.csv
      schema:
        - { name: id, type: string }
        - { name: amount, type: int }

  # comment between nodes
  - type: transform
    name: clean
    input: raw
    config:
      cxl: "emit id = id"

  - type: output
    name: out
    input: clean
    config:
      name: out
      type: csv
      path: out.csv

error_handling:
  strategy: continue

_notes:
  author: glitch
"#;

    fn node_names(cfg: &PipelineConfig) -> Vec<String> {
        cfg.nodes
            .iter()
            .map(|n| n.value.name().to_string())
            .collect()
    }

    /// REGRESSION TEST for issue #29 (P1 data-loss).
    ///
    /// Simulates the exact bug trigger: parse a pipeline with nodes, perform an
    /// inspector edit (`set_stage_notes`, the `EditSource::Inspector` path),
    /// run the inspector→YAML sync, and assert the result still parses to the
    /// SAME set of nodes (count + names). This FAILS against the old
    /// `serialize_yaml(config)` behavior (which drops the whole `nodes:`
    /// block); `assert_old_behavior_loses_nodes` below pins that failure.
    #[test]
    fn inspector_edit_preserves_all_nodes() {
        let before = parse_config(FIXTURE).expect("fixture parses");
        let before_names = node_names(&before);
        assert_eq!(before_names.len(), 3);

        // Inspector edit: add a stage note to the middle node.
        let mut edited = parse_config(FIXTURE).expect("fixture parses");
        edited.set_stage_notes("clean", Some(serde_json::json!({ "stage": "cleaned up" })));

        let patched = patch_yaml_preserving_nodes(FIXTURE, &edited);

        let after = parse_config(&patched).expect("patched YAML re-parses");
        assert_eq!(
            node_names(&after),
            before_names,
            "every node must survive the inspector→YAML sync (issue #29)"
        );
        assert_eq!(
            after.stage_notes("clean"),
            Some(&serde_json::json!({ "stage": "cleaned up" })),
            "the inspector edit must be reflected in the YAML"
        );
    }

    /// Proves the OLD `serialize_yaml(config)` path is the bug: serializing the
    /// edited config through the engine drops the `nodes:` block entirely, so
    /// the round-tripped document has ZERO nodes. This is the behavior the
    /// regression test above guards against.
    #[test]
    fn assert_old_behavior_loses_nodes() {
        let mut edited = parse_config(FIXTURE).expect("fixture parses");
        edited.set_stage_notes("clean", Some(serde_json::json!({ "stage": "x" })));

        // The pre-fix code did exactly this.
        let old_output = clinker_core::yaml::to_string(&edited).expect("engine serializes");
        let reparsed = parse_config(&old_output);

        // Engine output has no `nodes:` block, so it either fails validation or
        // parses to an empty node set — either way the nodes are gone.
        let node_count = reparsed.map(|c| c.nodes.len()).unwrap_or(0);
        assert_eq!(
            node_count, 0,
            "old serialize_yaml(config) path must drop all nodes (the #29 bug)"
        );
    }

    /// Focused span-patch test: editing ONE node must leave every other node's
    /// bytes — and a comment — untouched.
    #[test]
    fn span_patch_leaves_other_nodes_and_comments_verbatim() {
        let mut edited = parse_config(FIXTURE).expect("fixture parses");
        edited.set_stage_notes("clean", Some(serde_json::json!({ "stage": "note" })));

        let patched = patch_yaml_preserving_nodes(FIXTURE, &edited);

        // Untouched source node retains its inline comment verbatim.
        assert!(
            patched.contains("    name: raw          # inline comment on raw\n"),
            "the unedited source node must be byte-identical, comment and all:\n{patched}"
        );
        // The whole source-node block survives verbatim (schema list, etc.).
        assert!(patched.contains("        - { name: amount, type: int }\n"));
        // The untouched output node block survives verbatim.
        assert!(patched.contains("  - type: output\n    name: out\n    input: clean\n"));
        // Leading comment, trailing sections survive.
        assert!(patched.contains("# a leading comment\n"));
        assert!(patched.contains("error_handling:\n  strategy: continue\n"));
        assert!(patched.contains("_notes:\n  author: glitch\n"));
        // The edited node now carries its note.
        assert!(patched.contains("stage: note"));
    }

    /// Editing a node that is NOT the first or last still preserves the head
    /// (`pipeline:` + leading comment) and the `nodes:` line verbatim.
    #[test]
    fn head_is_preserved_verbatim() {
        let mut edited = parse_config(FIXTURE).expect("fixture parses");
        edited.set_stage_notes("clean", Some(serde_json::json!({ "stage": "y" })));
        let patched = patch_yaml_preserving_nodes(FIXTURE, &edited);
        assert!(patched.starts_with("pipeline:\n  name: demo\n\n# a leading comment\nnodes:\n"));
    }

    /// The full-rebuild fallback must always emit a real `nodes:` block — never
    /// node-less output even for an in-memory config with no source text.
    #[test]
    fn full_serializer_never_drops_nodes() {
        let cfg = parse_config(FIXTURE).expect("fixture parses");
        let out = serialize_yaml_full(&cfg);
        let reparsed = parse_config(&out).expect("full-serializer output re-parses");
        assert_eq!(node_names(&reparsed), node_names(&cfg));
        assert!(
            out.contains("nodes:\n"),
            "must contain a nodes block:\n{out}"
        );
    }

    /// An unparsable current buffer routes to the full-rebuild fallback rather
    /// than panicking or returning node-less text.
    #[test]
    fn unparsable_text_falls_back_to_full_serializer() {
        let cfg = parse_config(FIXTURE).expect("fixture parses");
        let garbage = "::: not yaml :::\n\t- broken";
        let out = patch_yaml_preserving_nodes(garbage, &cfg);
        let reparsed = parse_config(&out).expect("fallback output re-parses");
        assert_eq!(node_names(&reparsed), node_names(&cfg));
    }

    /// A node-count divergence (structural add/remove, not an in-place edit)
    /// routes to the fallback, which still preserves every node in `config`.
    #[test]
    fn node_count_divergence_falls_back() {
        // `current_yaml` has 3 nodes; `config` (after dropping one) has 2.
        let mut cfg = parse_config(FIXTURE).expect("fixture parses");
        cfg.nodes.remove(1);
        let out = patch_yaml_preserving_nodes(FIXTURE, &cfg);
        let reparsed = parse_config(&out).expect("fallback output re-parses");
        assert_eq!(reparsed.nodes.len(), 2);
        assert_eq!(
            node_names(&reparsed),
            vec!["raw".to_string(), "out".to_string()]
        );
    }

    /// A no-op inspector edit (nothing actually changed) returns the document
    /// byte-for-byte — no node is reformatted.
    #[test]
    fn noop_edit_returns_document_verbatim() {
        let cfg = parse_config(FIXTURE).expect("fixture parses");
        let out = patch_yaml_preserving_nodes(FIXTURE, &cfg);
        assert_eq!(out, FIXTURE, "a no-op edit must not perturb the document");
    }

    /// Editing a node must NOT delete an inter-node comment that sits between
    /// the edited node's body and the next node. A node's source region runs up
    /// to the next node's line-start, so such a comment is byte-wise inside the
    /// edited node's region even though it visually belongs "before the next
    /// node"; re-serializing the whole region used to drop it. Edit the FIRST
    /// node (whose region contains the `# comment between nodes` line) and
    /// assert the comment survives.
    #[test]
    fn editing_a_node_preserves_following_inter_node_comment() {
        let mut edited = parse_config(FIXTURE).expect("fixture parses");
        edited.set_stage_notes("raw", Some(serde_json::json!({ "edited": true })));
        let patched = patch_yaml_preserving_nodes(FIXTURE, &edited);

        assert!(
            patched.contains("# comment between nodes"),
            "an inter-node comment after the edited node must survive:\n{patched}"
        );
        let after = parse_config(&patched).expect("patched re-parses");
        assert_eq!(node_names(&after), node_names(&edited));
        assert_eq!(
            after.stage_notes("raw"),
            Some(&serde_json::json!({ "edited": true }))
        );
    }

    /// Editing the LAST node must preserve a trailing comment that follows it
    /// with no top-level sibling key after (the region extends to EOF). The
    /// trailing-gutter split keeps that comment verbatim.
    #[test]
    fn editing_last_node_preserves_trailing_comment() {
        let src = "pipeline:\n  name: demo\nnodes:\n  - type: source\n    name: a\n    \
config:\n      name: a\n      type: csv\n      path: a.csv\n      schema:\n        \
- { name: id, type: string }\n# trailing comment after last node\n";
        let mut edited = parse_config(src).expect("fixture parses");
        edited.set_stage_notes("a", Some(serde_json::json!({ "n": 1 })));
        let patched = patch_yaml_preserving_nodes(src, &edited);

        assert!(
            patched.contains("# trailing comment after last node"),
            "a trailing comment after the last (edited) node must survive:\n{patched}"
        );
        assert_eq!(parse_config(&patched).expect("re-parses").nodes.len(), 1);
    }

    /// The full-rebuild fallback must emit an explicit `nodes: []` for an empty
    /// node set so the output still re-parses — `nodes:` is a required engine
    /// key, and node-less output would fail to load (the very class of failure
    /// this module exists to prevent).
    #[test]
    fn full_serializer_emits_empty_nodes_block() {
        let src = "pipeline:\n  name: demo\nnodes: []\nerror_handling:\n  strategy: continue\n";
        let cfg = parse_config(src).expect("empty-nodes fixture parses");
        assert!(cfg.nodes.is_empty());
        let out = serialize_yaml_full(&cfg);
        assert!(
            out.contains("nodes: []") || out.contains("nodes:\n"),
            "must emit a nodes key for an empty node set:\n{out}"
        );
        let reparsed = parse_config(&out).expect("empty-nodes fallback output re-parses");
        assert!(reparsed.nodes.is_empty());
    }
}
