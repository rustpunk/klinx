use std::collections::{HashMap, HashSet, VecDeque};

use clinker_plan::config::{PipelineConfig, PipelineNode};

use crate::autodoc::{CxlStatementKind, generate_stage_doc};
use crate::notes::parse_notes;
use crate::pipeline_view::{
    EdgeNature, FieldEdgeKind, FieldKind, PipelineView, Precision, RoleEdge, StagePortSide,
    StageView,
};
use crate::state::{ChannelViewMode, SelectedField};

#[derive(Clone, Debug, PartialEq)]
pub enum InspectorSelection {
    Node(String),
    Field(SelectedField),
}

#[derive(Clone, Debug, PartialEq)]
pub enum SelectedInspectorModel {
    Node(Box<NodeInspectorModel>),
    Field(Box<FieldInspectorModel>),
    Missing(MissingInspectorModel),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MissingInspectorModel {
    pub kind_label: &'static str,
    pub kind_attr: &'static str,
    pub label: String,
    pub stage_id: Option<String>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeInspectorModel {
    pub stage_id: String,
    pub label: String,
    pub kind_label: &'static str,
    pub kind_attr: &'static str,
    pub status_chips: Vec<StatusChip>,
    pub overview: Vec<InspectorFact>,
    pub sections: Vec<InspectorSection>,
    pub diagnostics: Vec<InspectorDiagnostic>,
    pub composition_params: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldInspectorModel {
    pub selection: SelectedField,
    pub label: String,
    pub stage_label: String,
    pub stage_kind_label: &'static str,
    pub stage_kind_attr: &'static str,
    pub field_name: String,
    pub field_kind_label: &'static str,
    pub field_kind_attr: &'static str,
    pub field_type: String,
    pub badges: Vec<String>,
    pub context: Vec<InspectorFact>,
    pub explanation: String,
    pub annotation: Option<String>,
    pub cxl_mentions: Vec<CxlMentionView>,
    /// Upstream lineage as a hop-by-hop TREE rooted at this field (#153). Each
    /// [`TraceNode`] names the transform + edge kind + precision of the hop that
    /// reached it; its `children` are the hops one step further upstream. The root
    /// (the selected field, hop 0) is implicit — the top-level `Vec` is its direct
    /// hops (hop 1). Replaced the former flat `Vec<TraceEndpointView>` so the panel
    /// can render parent→child topology instead of a sorted list.
    pub upstream: Vec<TraceNode>,
    /// Downstream lineage as a hop-by-hop tree, mirroring [`Self::upstream`] in the
    /// impact direction (#153).
    pub downstream: Vec<TraceNode>,
    /// The same upstream lineage as [`Self::upstream`], but built with INDIRECT
    /// influence edges excluded (#153) — the tree behind the Inspector's INDIRECT
    /// toggle in its "off" state. Built by the same `trace_tree` walk with
    /// `include_indirect = false`, NOT by pruning [`Self::upstream`]: an endpoint
    /// reached by BOTH a DIRECT carry and an INDIRECT influence (e.g. a Combine join
    /// key) survives here via its carry edge, correctly tagged DIRECT, whereas
    /// pruning the worst-precision-deduped full tree would drop it. Precomputed so the
    /// panel toggle selects between two ready trees without holding pipeline state.
    pub upstream_direct: Vec<TraceNode>,
    /// Downstream counterpart of [`Self::upstream_direct`] (#153).
    pub downstream_direct: Vec<TraceNode>,
    pub role_usages: Vec<RoleUsageView>,
    /// The field's OWN lineage precision tier (#148) — the producer-side value from
    /// `FieldRow::lineage_precision`, NOT a transitive trace fold — surfaced as the
    /// Inspector's per-field precision badge. Reading the row's own value keeps this
    /// badge in agreement with the canvas node-corner and the row model, and avoids
    /// over-degrading every field upstream of a single influence edge. Per-hop
    /// precision (`TraceEndpointView::precision`) still shows each hop's own edge
    /// tier, so an approximation is visible exactly where it occurs. Replaced the
    /// former binary `lineage_unavailable_reason: Option<String>`; the old "no
    /// lineage edges" empty-state is folded into
    /// [`FieldInspectorModel::precision_reason`] / `lineage_empty`.
    pub lineage_precision: Precision,
    /// Plain-language explanation for `lineage_precision`. For a field with NO
    /// lineage edges at all this carries the preserved empty-state message ("No
    /// field-level lineage edges mention this field in the current view."), so the
    /// acceptance guard against regressing that message still holds.
    pub precision_reason: String,
    /// Whether this field has no lineage edges at all (no upstream/downstream/role
    /// uses), so the body renders the empty-state presentation rather than a
    /// degraded-precision warning. Kept distinct from precision so an `Exact` field
    /// with edges is never confused with an edgeless field.
    pub lineage_empty: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StatusChip {
    pub label: String,
    pub tone: StatusTone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusTone {
    Ok,
    Info,
    Warn,
    Error,
}

impl StatusTone {
    pub fn as_attr(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InspectorFact {
    pub label: String,
    pub value: String,
}

impl InspectorFact {
    fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InspectorSection {
    pub title: &'static str,
    pub facts: Vec<InspectorFact>,
    pub rows: Vec<InspectorRow>,
    pub unavailable: Option<String>,
}

impl InspectorSection {
    fn with_facts(title: &'static str, facts: Vec<InspectorFact>) -> Self {
        Self {
            title,
            facts,
            rows: Vec::new(),
            unavailable: None,
        }
    }

    fn unavailable(title: &'static str, reason: impl Into<String>) -> Self {
        Self {
            title,
            facts: Vec::new(),
            rows: Vec::new(),
            unavailable: Some(reason.into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InspectorRow {
    pub label: String,
    pub value: String,
    pub tone: Option<StatusTone>,
}

impl InspectorRow {
    fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            tone: None,
        }
    }

    fn toned(label: impl Into<String>, value: impl Into<String>, tone: StatusTone) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            tone: Some(tone),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InspectorDiagnostic {
    pub label: String,
    pub message: String,
    pub tone: StatusTone,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CxlMentionView {
    pub kind: &'static str,
    pub expression: String,
    pub reads: String,
    pub writes: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TraceEndpointView {
    pub stage_id: String,
    pub stage_label: String,
    pub stage_kind_label: &'static str,
    pub stage_kind_attr: &'static str,
    pub field_name: String,
    pub edge_kind_label: &'static str,
    pub edge_kind_attr: &'static str,
    /// The precision tier of the lineage edge taken to reach this hop (#148),
    /// rendered as the per-hop precision badge alongside `edge_kind_label`. Carried
    /// as the [`Precision`] enum (not a pre-baked slug string) so the panel derives
    /// its label/attr via `precision_label`/`precision_attr` — no lossy
    /// string→enum round-trip. When an endpoint is reachable by several edges the
    /// WORST (least-precise) edge's precision is kept (see [`trace_tree`]), so
    /// an Exact carry can never mask a co-incident Approximate influence.
    pub precision: Precision,
    pub hop: usize,
}

impl TraceEndpointView {
    /// The canvas field this trace hop points at. Selecting it drives the shared
    /// [`SelectedField`] state, which both the canvas reveal effect and the
    /// inspector read — so clicking a hop in the Inspector navigates the canvas
    /// to that field (#151). `stage_id` + `field_name` are the field identity,
    /// carried verbatim from the trace BFS, so no lookup is needed.
    pub fn to_selected_field(&self) -> SelectedField {
        SelectedField::new(self.stage_id.clone(), self.field_name.clone())
    }
}

/// One node in a hop-by-hop lineage trace TREE (#153). The selected field is the
/// implicit root (hop 0); each node's `endpoint` names the transform, edge kind,
/// and precision of the single hop that reached it, and `children` are the hops
/// one step further out (upstream or downstream depending on the trace direction).
///
/// Carrying the parent→child topology — rather than the former flat, hop-sorted
/// `Vec<TraceEndpointView>` — lets the Inspector render an expandable tree that
/// attributes each hop to the responsible transform, instead of a list that
/// discards which earlier hop a deeper one descends from.
#[derive(Clone, Debug, PartialEq)]
pub struct TraceNode {
    pub endpoint: TraceEndpointView,
    /// The CXL statement(s) on this hop's OWN stage that mention this hop's field
    /// (#153) — the responsible-transform enrichment. Empty when the hop's stage
    /// has no CXL analysis (Route/Aggregate/Merge) or no statement touches the
    /// field; in that case the edge kind + precision badge IS the attribution.
    pub cxl_mentions: Vec<CxlMentionView>,
    pub children: Vec<TraceNode>,
}

impl TraceNode {
    /// Total node count in this subtree, including the node itself (#153). The
    /// LINEAGE summary counts every traced hop, not just the direct (hop-1) ones,
    /// so the count matches the former flat list's length.
    fn count(&self) -> usize {
        1 + self.children.iter().map(TraceNode::count).sum::<usize>()
    }
}

/// Total node count across a forest of trace trees (#153).
pub fn count_trace_nodes(nodes: &[TraceNode]) -> usize {
    nodes.iter().map(TraceNode::count).sum()
}

#[derive(Clone, Debug, PartialEq)]
pub struct RoleUsageView {
    pub stage_label: String,
    pub stage_kind_label: &'static str,
    pub stage_kind_attr: &'static str,
    pub port_label: String,
    pub role: String,
    pub edge_kind_label: &'static str,
    pub edge_kind_attr: &'static str,
}

#[derive(Clone, Copy)]
enum TraceDirection {
    Upstream,
    Downstream,
}

pub struct InspectorBuildContext<'a> {
    pub view: &'a PipelineView,
    pub config: Option<&'a PipelineConfig>,
    pub channel_mode: ChannelViewMode,
    pub compiled_plan_available: bool,
    pub visible_errors: &'a [String],
    pub schema_warnings: &'a [String],
}

pub fn build_selected_inspector(
    selection: InspectorSelection,
    ctx: InspectorBuildContext<'_>,
) -> SelectedInspectorModel {
    match selection {
        InspectorSelection::Node(stage_id) => build_node_detail(&stage_id, ctx),
        InspectorSelection::Field(selection) => {
            build_field_detail(ctx.view, ctx.config, &selection)
                .map(|field| SelectedInspectorModel::Field(Box::new(field)))
                .unwrap_or_else(|| {
                    SelectedInspectorModel::Missing(MissingInspectorModel {
                        kind_label: "FIELD",
                        kind_attr: "error",
                        label: format!("{}.{}", selection.stage_id, selection.field_name),
                        stage_id: Some(selection.stage_id),
                        reason: "This field is not present in the current canvas view.".to_string(),
                    })
                })
        }
    }
}

fn build_node_detail(stage_id: &str, ctx: InspectorBuildContext<'_>) -> SelectedInspectorModel {
    let stage = ctx.view.stages.iter().find(|stage| stage.id == stage_id);
    let node = ctx.config.and_then(|config| {
        config
            .nodes
            .iter()
            .find(|node| node.value.name() == stage_id)
            .map(|node| &node.value)
    });

    let (kind_label, kind_attr, label) = match (stage, node) {
        (Some(stage), _) => (
            stage.kind.badge_label(),
            stage.kind.kind_attr(),
            stage.label.clone(),
        ),
        (None, Some(node)) => (
            crate::pipeline_view::stage_kind_for_node(node).badge_label(),
            crate::pipeline_view::stage_kind_for_node(node).kind_attr(),
            stage_id.to_string(),
        ),
        (None, None) => {
            return SelectedInspectorModel::Missing(MissingInspectorModel {
                kind_label: "NODE",
                kind_attr: "error",
                label: stage_id.to_string(),
                stage_id: Some(stage_id.to_string()),
                reason: "This node is not present in the current parsed pipeline view.".to_string(),
            });
        }
    };

    let notes = ctx
        .config
        .map(|config| parse_notes(config.stage_notes(stage_id)))
        .unwrap_or_default();
    let doc = ctx
        .config
        .and_then(|config| generate_stage_doc(config, stage_id));
    let mut overview = vec![
        InspectorFact::new("name", stage_id),
        InspectorFact::new("type", kind_label.to_ascii_lowercase()),
    ];
    if let Some(stage) = stage {
        if !stage.subtitle.is_empty() {
            overview.push(InspectorFact::new("summary", stage.subtitle.clone()));
        }
        if let Some(description) = stage.description.as_ref().filter(|value| !value.is_empty()) {
            overview.push(InspectorFact::new("description", description.clone()));
        }
    }
    if let Some(doc) = &doc
        && !doc.summary.is_empty()
    {
        overview.push(InspectorFact::new("autodoc", doc.summary.clone()));
    }
    if !notes.stage_note.is_empty() {
        overview.push(InspectorFact::new("note", notes.stage_note.clone()));
    }

    // Validate the node's CXL block once and share the result between the
    // status diagnostics (which drive the node chip) and the CXL section (#141).
    let cxl_errors = node
        .and_then(node_cxl_source)
        .map(|source| crate::cxl_bridge::validate_expr(&source).errors)
        .unwrap_or_default();
    let diagnostics = node_diagnostics(stage, &cxl_errors, ctx.visible_errors, ctx.schema_warnings);
    let mut status_chips = status_chips(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.tone == StatusTone::Error),
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.tone == StatusTone::Warn),
        ctx.channel_mode,
        ctx.compiled_plan_available,
    );
    if stage.is_none() {
        status_chips.push(StatusChip {
            label: "no canvas stage".to_string(),
            tone: StatusTone::Warn,
        });
    }

    let sections = vec![
        topology_section(ctx.view, stage_id),
        node_logic_section(node, stage),
        fields_section(stage),
        branches_section(stage),
        role_ports_section(stage),
        cxl_section(stage_id, node, doc.as_ref(), &cxl_errors),
        contract_section(doc.as_ref()),
        channel_section(ctx.channel_mode, ctx.compiled_plan_available),
    ];

    let composition_params = match node {
        Some(PipelineNode::Composition { config, .. }) => {
            let mut params = config.keys().cloned().collect::<Vec<_>>();
            params.sort();
            params
        }
        _ => Vec::new(),
    };

    SelectedInspectorModel::Node(Box::new(NodeInspectorModel {
        stage_id: stage_id.to_string(),
        label,
        kind_label,
        kind_attr,
        status_chips,
        overview,
        sections,
        diagnostics,
        composition_params,
    }))
}

fn status_chips(
    has_error: bool,
    has_warning: bool,
    channel_mode: ChannelViewMode,
    compiled_plan_available: bool,
) -> Vec<StatusChip> {
    let mut chips = Vec::new();
    chips.push(StatusChip {
        label: if has_error {
            "errors"
        } else if has_warning {
            "warnings"
        } else {
            "ok"
        }
        .to_string(),
        tone: if has_error {
            StatusTone::Error
        } else if has_warning {
            StatusTone::Warn
        } else {
            StatusTone::Ok
        },
    });
    chips.push(StatusChip {
        label: match channel_mode {
            ChannelViewMode::Raw => "raw view",
            ChannelViewMode::Resolved => "resolved view",
        }
        .to_string(),
        tone: StatusTone::Info,
    });
    if compiled_plan_available {
        chips.push(StatusChip {
            label: "compiled plan".to_string(),
            tone: StatusTone::Info,
        });
    }
    chips
}

/// Render a CXL parse diagnostic as a single human line: the parser message,
/// followed by ` → {how_to_fix}` when the parser offers an actionable fix.
fn cxl_diagnostic_message(diagnostic: &crate::cxl_bridge::CxlDiagnostic) -> String {
    if diagnostic.how_to_fix.is_empty() {
        diagnostic.message.clone()
    } else {
        format!("{} \u{2192} {}", diagnostic.message, diagnostic.how_to_fix)
    }
}

fn node_diagnostics(
    stage: Option<&StageView>,
    cxl_errors: &[crate::cxl_bridge::CxlDiagnostic],
    visible_errors: &[String],
    schema_warnings: &[String],
) -> Vec<InspectorDiagnostic> {
    let mut diagnostics = Vec::new();
    if let Some(error) = stage.and_then(|stage| stage.error_message.as_ref()) {
        diagnostics.push(InspectorDiagnostic {
            label: "stage".to_string(),
            message: error.clone(),
            tone: StatusTone::Error,
        });
    }
    // Edit-time CXL syntax validation: a malformed `cxl:` block surfaces as an
    // Error diagnostic, which flips the node status chip off "ok" (#141).
    diagnostics.extend(cxl_errors.iter().map(|diagnostic| InspectorDiagnostic {
        label: "cxl".to_string(),
        message: cxl_diagnostic_message(diagnostic),
        tone: StatusTone::Error,
    }));
    diagnostics.extend(visible_errors.iter().map(|message| InspectorDiagnostic {
        label: "parse".to_string(),
        message: message.clone(),
        tone: StatusTone::Error,
    }));
    diagnostics.extend(schema_warnings.iter().map(|message| InspectorDiagnostic {
        label: "schema".to_string(),
        message: message.clone(),
        tone: StatusTone::Warn,
    }));
    diagnostics
}

fn topology_section(view: &PipelineView, stage_id: &str) -> InspectorSection {
    let Some(index) = view.stages.iter().position(|stage| stage.id == stage_id) else {
        return InspectorSection::unavailable(
            "TOPOLOGY",
            "Topology is unavailable because the node is not in the current view.",
        );
    };

    let mut rows = Vec::new();
    for connection in view
        .connections
        .iter()
        .filter(|connection| connection.to == index)
    {
        if let Some(from) = view.stages.get(connection.from) {
            let branch = connection
                .from_branch
                .and_then(|branch| from.branches.get(branch))
                .map(|branch| format!(".{}", branch.name))
                .unwrap_or_default();
            rows.push(InspectorRow::new(
                "input",
                format!("{}{}", from.label, branch),
            ));
        }
    }
    for connection in view
        .connections
        .iter()
        .filter(|connection| connection.from == index)
    {
        if let Some(to) = view.stages.get(connection.to) {
            let branch = connection
                .from_branch
                .and_then(|branch| view.stages[index].branches.get(branch))
                .map(|branch| format!("{} -> {}", branch.name, to.label))
                .unwrap_or_else(|| to.label.clone());
            rows.push(InspectorRow::new("output", branch));
        }
    }

    if rows.is_empty() {
        InspectorSection::unavailable(
            "TOPOLOGY",
            "No node-level connections are present for this selection.",
        )
    } else {
        InspectorSection {
            title: "TOPOLOGY",
            facts: Vec::new(),
            rows,
            unavailable: None,
        }
    }
}

fn node_logic_section(node: Option<&PipelineNode>, stage: Option<&StageView>) -> InspectorSection {
    let Some(node) = node else {
        return InspectorSection::unavailable(
            "LOGIC",
            "Node config is unavailable while the YAML is partially parsed.",
        );
    };

    match node {
        PipelineNode::Source { config, .. } => InspectorSection::with_facts(
            "LOGIC",
            vec![
                InspectorFact::new("kind", "source"),
                InspectorFact::new("target", config.source.display_target()),
            ],
        ),
        PipelineNode::Transform { config, .. } => InspectorSection::with_facts(
            "LOGIC",
            vec![
                InspectorFact::new("kind", "transform"),
                InspectorFact::new(
                    "cxl bytes",
                    config.cxl.as_ref().to_string().len().to_string(),
                ),
            ],
        ),
        PipelineNode::Aggregate { config, .. } => InspectorSection::with_facts(
            "LOGIC",
            vec![
                InspectorFact::new("kind", "aggregate"),
                InspectorFact::new("group_by", join_or_unavailable(&config.group_by)),
                InspectorFact::new(
                    "cxl bytes",
                    config.cxl.as_ref().to_string().len().to_string(),
                ),
            ],
        ),
        PipelineNode::Route { config, .. } => {
            let mut rows = config
                .conditions
                .iter()
                .map(|(branch, predicate)| {
                    InspectorRow::new(format!("branch {branch}"), predicate.as_ref().to_string())
                })
                .collect::<Vec<_>>();
            rows.push(InspectorRow::toned(
                "default",
                config.default.clone(),
                StatusTone::Info,
            ));
            InspectorSection {
                title: "LOGIC",
                facts: vec![InspectorFact::new(
                    "route branches",
                    config.conditions.len().to_string(),
                )],
                rows,
                unavailable: None,
            }
        }
        PipelineNode::Merge { header, .. } => InspectorSection::with_facts(
            "LOGIC",
            vec![InspectorFact::new(
                "inputs",
                header.inputs.len().to_string(),
            )],
        ),
        PipelineNode::Combine { header, config } => InspectorSection::with_facts(
            "LOGIC",
            vec![
                InspectorFact::new("inputs", header.input.len().to_string()),
                InspectorFact::new(
                    "cxl bytes",
                    config.cxl.as_ref().to_string().len().to_string(),
                ),
            ],
        ),
        PipelineNode::Output { config, .. } => InspectorSection::with_facts(
            "LOGIC",
            vec![
                InspectorFact::new("kind", "output"),
                InspectorFact::new("path", config.output.path.clone()),
            ],
        ),
        PipelineNode::Composition { r#use, config, .. } => InspectorSection::with_facts(
            "LOGIC",
            vec![
                InspectorFact::new("use", r#use.display().to_string()),
                InspectorFact::new("overrides", config.len().to_string()),
            ],
        ),
        PipelineNode::Reshape { config, .. } => operator_body_section(
            "LOGIC",
            "reshape",
            vec![
                InspectorFact::new("partition_by", join_or_unavailable(&config.partition_by)),
                InspectorFact::new("order_by", order_by_summary(&config.order_by)),
            ],
            config
                .rules
                .iter()
                .map(|rule| {
                    InspectorRow::new(
                        rule.name.clone(),
                        format!("{} | {}", rule.when.as_ref(), reshape_rule_actions(rule)),
                    )
                })
                .collect(),
        ),
        PipelineNode::Cull { config, .. } => operator_body_section(
            "LOGIC",
            "cull",
            vec![
                InspectorFact::new("partition_by", join_or_unavailable(&config.partition_by)),
                InspectorFact::new("order_by", order_by_summary(&config.order_by)),
                InspectorFact::new("removed_to", config.removed_to.clone()),
            ],
            config
                .rules
                .iter()
                .map(|rule| {
                    InspectorRow::new(rule.name.clone(), rule.drop_group_when.as_ref().to_string())
                })
                .collect(),
        ),
        PipelineNode::Envelope { config, .. } => InspectorSection::with_facts(
            "LOGIC",
            vec![
                InspectorFact::new("kind", "envelope"),
                InspectorFact::new("strategy", envelope_strategy_name(&config.strategy)),
            ],
        ),
    }
    .with_stage_fallback(stage)
}

trait SectionStageFallback {
    fn with_stage_fallback(self, stage: Option<&StageView>) -> Self;
}

impl SectionStageFallback for InspectorSection {
    fn with_stage_fallback(mut self, stage: Option<&StageView>) -> Self {
        if self.facts.is_empty()
            && self.rows.is_empty()
            && let Some(stage) = stage
            && !stage.subtitle.is_empty()
        {
            self.facts
                .push(InspectorFact::new("stage", stage.subtitle.clone()));
        }
        self
    }
}

fn fields_section(stage: Option<&StageView>) -> InspectorSection {
    let Some(stage) = stage else {
        return InspectorSection::unavailable(
            "FIELDS",
            "Fields are unavailable because this node is not in the current view.",
        );
    };
    if stage.fields.is_empty() {
        return InspectorSection::unavailable(
            "FIELDS",
            "No output field rows are available. The current view may lack schema or typed row information for this node.",
        );
    }
    InspectorSection {
        title: "FIELDS",
        facts: vec![InspectorFact::new(
            "produced fields",
            stage.fields.len().to_string(),
        )],
        rows: stage
            .fields
            .iter()
            .map(|field| {
                InspectorRow::new(
                    field_kind_label(field.kind),
                    format!(
                        "{}{}",
                        field.name,
                        field
                            .ty
                            .as_ref()
                            .map(|ty| format!(": {ty}"))
                            .unwrap_or_default()
                    ),
                )
            })
            .collect(),
        unavailable: None,
    }
}

fn branches_section(stage: Option<&StageView>) -> InspectorSection {
    let Some(stage) = stage else {
        return InspectorSection::unavailable(
            "BRANCH PORTS",
            "Branch ports are unavailable because this node is not in the current view.",
        );
    };
    if stage.branches.is_empty() {
        return InspectorSection::unavailable(
            "BRANCH PORTS",
            "This node has no route/default side-output ports.",
        );
    }
    InspectorSection {
        title: "BRANCH PORTS",
        facts: Vec::new(),
        rows: stage
            .branches
            .iter()
            .map(|branch| {
                if branch.is_default {
                    InspectorRow::toned(&branch.name, "default fallback", StatusTone::Info)
                } else {
                    InspectorRow::new(
                        &branch.name,
                        branch
                            .predicate
                            .as_deref()
                            .unwrap_or("predicate unavailable"),
                    )
                }
            })
            .collect(),
        unavailable: None,
    }
}

fn role_ports_section(stage: Option<&StageView>) -> InspectorSection {
    let Some(stage) = stage else {
        return InspectorSection::unavailable(
            "ROLE PORTS",
            "Role ports are unavailable because this node is not in the current view.",
        );
    };
    let rows = stage
        .role_ports
        .iter()
        .map(|port| {
            let side = match port.side {
                StagePortSide::Input => "input",
                StagePortSide::Output => "output",
            };
            InspectorRow::new(format!("{side} {}", port.role), port.label.clone())
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        InspectorSection::unavailable(
            "ROLE PORTS",
            "No semantic role ports were inferred for this node.",
        )
    } else {
        InspectorSection {
            title: "ROLE PORTS",
            facts: Vec::new(),
            rows,
            unavailable: None,
        }
    }
}

fn cxl_section(
    stage_id: &str,
    node: Option<&PipelineNode>,
    doc: Option<&crate::autodoc::StageDoc>,
    cxl_errors: &[crate::cxl_bridge::CxlDiagnostic],
) -> InspectorSection {
    let Some(cxl_source) = node.and_then(node_cxl_source) else {
        return InspectorSection::unavailable("CXL", "This node has no top-level CXL block.");
    };
    let mut rows = Vec::new();
    // Surface syntax errors at the top of the section so a malformed block is
    // visible even when statement analysis yields nothing (#141). Errors are
    // validated once in `build_node_detail` and shared with the status chip.
    for diagnostic in cxl_errors {
        rows.push(InspectorRow::toned(
            "error",
            cxl_diagnostic_message(diagnostic),
            StatusTone::Error,
        ));
    }
    if let Some(analysis) = doc.and_then(|doc| doc.cxl_analysis.as_ref()) {
        for statement in &analysis.statements {
            rows.push(InspectorRow::new(
                statement.kind.label(),
                format!(
                    "reads [{}] writes [{}] :: {}",
                    join_or_unavailable(&statement.field_refs),
                    statement
                        .output_field
                        .as_deref()
                        .unwrap_or("no output field"),
                    statement.expression
                ),
            ));
        }
    }
    if rows.is_empty() {
        return InspectorSection {
            title: "CXL",
            facts: vec![InspectorFact::new(
                "source",
                format!("{} byte(s)", cxl_source.len()),
            )],
            rows,
            unavailable: Some(format!(
                "Statement reads/writes are not available for {stage_id} in this view."
            )),
        };
    }
    InspectorSection {
        title: "CXL",
        facts: vec![InspectorFact::new(
            "source",
            format!("{} byte(s)", cxl_source.len()),
        )],
        rows,
        unavailable: None,
    }
}

fn node_cxl_source(node: &PipelineNode) -> Option<String> {
    match node {
        PipelineNode::Transform { config, .. } => Some(config.cxl.as_ref().to_string()),
        PipelineNode::Aggregate { config, .. } => Some(config.cxl.as_ref().to_string()),
        PipelineNode::Combine { config, .. } => Some(config.cxl.as_ref().to_string()),
        _ => None,
    }
}

fn contract_section(doc: Option<&crate::autodoc::StageDoc>) -> InspectorSection {
    let Some(doc) = doc else {
        return InspectorSection::unavailable(
            "CONTRACT",
            "Contract documentation is not available for this node kind.",
        );
    };
    if let Some(contract) = &doc.contract {
        return InspectorSection::with_facts(
            "CONTRACT",
            vec![
                InspectorFact::new("composition", contract.composition_name.clone()),
                InspectorFact::new(
                    "version",
                    contract
                        .version
                        .clone()
                        .unwrap_or_else(|| "not specified".to_string()),
                ),
                InspectorFact::new("requires", contract.requires.len().to_string()),
                InspectorFact::new("produces", contract.produces.len().to_string()),
            ],
        );
    }
    if let Some(schema) = &doc.schema {
        return InspectorSection::with_facts(
            "CONTRACT",
            vec![InspectorFact::new(
                "schema fields",
                schema.fields.len().to_string(),
            )],
        );
    }
    InspectorSection::unavailable(
        "CONTRACT",
        "No schema or composition contract facts are available for this stage.",
    )
}

fn channel_section(
    channel_mode: ChannelViewMode,
    compiled_plan_available: bool,
) -> InspectorSection {
    let mut facts = vec![InspectorFact::new(
        "view",
        match channel_mode {
            ChannelViewMode::Raw => "raw authored YAML",
            ChannelViewMode::Resolved => "resolved channel overlay",
        },
    )];
    facts.push(InspectorFact::new(
        "compiled plan",
        if compiled_plan_available {
            "available"
        } else {
            "not available"
        },
    ));
    if !compiled_plan_available {
        return InspectorSection {
            title: "CHANNEL / PROVENANCE",
            facts,
            rows: Vec::new(),
            unavailable: Some(
                "Override provenance cannot be shown until a compiled plan is available."
                    .to_string(),
            ),
        };
    }
    InspectorSection::with_facts("CHANNEL / PROVENANCE", facts)
}

fn operator_body_section(
    title: &'static str,
    kind: &'static str,
    mut facts: Vec<InspectorFact>,
    rows: Vec<InspectorRow>,
) -> InspectorSection {
    facts.insert(0, InspectorFact::new("kind", kind));
    InspectorSection {
        title,
        facts,
        rows,
        unavailable: None,
    }
}

fn build_field_detail(
    view: &PipelineView,
    config: Option<&PipelineConfig>,
    selection: &SelectedField,
) -> Option<FieldInspectorModel> {
    let stage_index = view
        .stages
        .iter()
        .position(|stage| stage.id == selection.stage_id)?;
    let stage = &view.stages[stage_index];
    let field = stage
        .fields
        .iter()
        .find(|field| field.name == selection.field_name)?;

    // The full trees include INDIRECT influence hops; the direct-only trees exclude
    // them (#153). Both pairs are built by the same `trace_tree` walk — the
    // direct-only pair with `include_indirect = false`, which (unlike a post-hoc
    // prune of the full tree) keeps an endpoint reached by both a DIRECT carry and an
    // INDIRECT influence, tagged by its surviving DIRECT edge. The Inspector toggle
    // selects between the two precomputed pairs, so the model holds no UI state.
    let mut upstream = trace_tree(
        view,
        stage_index,
        &selection.field_name,
        TraceDirection::Upstream,
        true,
    );
    let mut downstream = trace_tree(
        view,
        stage_index,
        &selection.field_name,
        TraceDirection::Downstream,
        true,
    );
    let mut upstream_direct = trace_tree(
        view,
        stage_index,
        &selection.field_name,
        TraceDirection::Upstream,
        false,
    );
    let mut downstream_direct = trace_tree(
        view,
        stage_index,
        &selection.field_name,
        TraceDirection::Downstream,
        false,
    );
    // Attach per-hop CXL attribution (#153) where the hop's stage carries CXL
    // analysis. Walks the assembled trees with `config` in scope, reusing one
    // `generate_stage_doc` cache across all four trees so a stage parsed once is
    // reused for every hop and both toggle states. A stage with no CXL
    // (Route/Aggregate/Merge) contributes nothing — the edge kind + precision badge
    // is the attribution there.
    if let Some(config) = config {
        let mut cache = StageDocCache::new(config);
        enrich_trace_cxl(&mut upstream, &mut cache);
        enrich_trace_cxl(&mut downstream, &mut cache);
        enrich_trace_cxl(&mut upstream_direct, &mut cache);
        enrich_trace_cxl(&mut downstream_direct, &mut cache);
    }
    let role_usages = role_usages(view, stage_index, &selection.field_name);
    let mut badges = Vec::new();
    if field.is_correlation_key {
        badges.push("correlation key".to_string());
    }
    // The aggregate (group-by) grain is now represented exactly once, as the
    // INDIRECT `GroupBy` edge (#147), not a separate row flag. A field carries
    // the grain badge when it is an endpoint of a `GroupBy` edge incident to this
    // stage — either the group-key output row this stage produces, or the
    // upstream column that drives it.
    if is_group_by_grain(view, stage_index, &selection.field_name) {
        badges.push("aggregate grain".to_string());
    }

    let notes = config
        .map(|config| parse_notes(config.stage_notes(&selection.stage_id)))
        .unwrap_or_default();
    let annotation = notes.field_annotations.get(&selection.field_name).cloned();
    let cxl_mentions = config
        .and_then(|config| generate_stage_doc(config, &selection.stage_id))
        .and_then(|doc| doc.cxl_analysis)
        .map(|analysis| cxl_mentions_for_field(&analysis.statements, &selection.field_name))
        .unwrap_or_default();

    // Field precision (#148) is the field's OWN row precision — the producer-side
    // value `derive_row_precision` already folded — NOT a transitive trace fold.
    // This keeps the Inspector field badge in agreement with the canvas node-corner
    // and the row model (all three read `FieldRow::lineage_precision`), and avoids
    // over-degrading every field transitively upstream of a single influence edge
    // into Approximate. The PER-HOP badges still show each hop's own edge precision
    // (see `TraceEndpointView::precision`), so the approximation is visible exactly
    // where it occurs without painting the whole upstream cone.
    let upstream_count = count_trace_nodes(&upstream);
    let downstream_count = count_trace_nodes(&downstream);
    let lineage_empty = upstream.is_empty() && downstream.is_empty() && role_usages.is_empty();
    let lineage_precision = field.lineage_precision;
    let precision_reason = if lineage_empty {
        // A field with NO lineage edges keeps the original empty-state message
        // verbatim (acceptance forbids regressing it), folded into the surfacing.
        "No field-level lineage edges mention this field in the current view.".to_string()
    } else if field.precision_reason.is_empty() {
        precision_default_reason(field.lineage_precision)
    } else {
        field.precision_reason.to_string()
    };

    Some(FieldInspectorModel {
        selection: selection.clone(),
        label: format!("{}.{}", stage.label, field.name),
        stage_label: stage.label.clone(),
        stage_kind_label: stage.kind.badge_label(),
        stage_kind_attr: stage.kind.kind_attr(),
        field_name: field.name.clone(),
        field_kind_label: field_kind_label(field.kind),
        field_kind_attr: field_kind_attr(field.kind),
        field_type: field.ty.clone().unwrap_or_else(|| "unknown".to_string()),
        badges,
        context: vec![
            InspectorFact::new("stage", stage.id.clone()),
            InspectorFact::new("stage type", stage.kind.badge_label()),
            InspectorFact::new("field kind", field_kind_label(field.kind)),
            InspectorFact::new(
                "lineage",
                format!(
                    "{} upstream / {} downstream / {} role uses",
                    upstream_count,
                    downstream_count,
                    role_usages.len()
                ),
            ),
        ],
        explanation: field_explanation(field.kind),
        annotation,
        cxl_mentions,
        upstream,
        downstream,
        upstream_direct,
        downstream_direct,
        role_usages,
        lineage_precision,
        precision_reason,
        lineage_empty,
    })
}

/// The default precision-reason for a field whose row carries no explicit reason
/// (#148) — an un-degraded `Exact` field gets a short affirmative note rather than
/// an empty string, so the Inspector badge always reads sensibly.
fn precision_default_reason(precision: Precision) -> String {
    match precision {
        Precision::Exact => "Exact: lineage carried or derived from resolved support.",
        Precision::Approximate => "Approximate: lineage is a sound over-approximation.",
        Precision::Unknown => "Unknown: lineage could not be computed.",
    }
    .to_string()
}

fn cxl_mentions_for_field(
    statements: &[crate::autodoc::CxlStatement],
    field_name: &str,
) -> Vec<CxlMentionView> {
    statements
        .iter()
        .filter(|statement| {
            statement.field_refs.iter().any(|field| field == field_name)
                || statement.output_field.as_deref() == Some(field_name)
        })
        .map(|statement| CxlMentionView {
            kind: cxl_kind_label(&statement.kind),
            expression: statement.expression.clone(),
            reads: join_or_unavailable(&statement.field_refs),
            writes: statement
                .output_field
                .clone()
                .unwrap_or_else(|| "no output field".to_string()),
        })
        .collect()
}

fn cxl_kind_label(kind: &CxlStatementKind) -> &'static str {
    kind.label()
}

/// Per-stage cache of `generate_stage_doc`'s CXL statements, so the trace-tree
/// enrichment parses each hop's stage at most once (#153). Several hops of one
/// trace can land on the same Transform/Aggregate stage; without the cache each
/// would re-run `generate_stage_doc` (which re-parses the stage's CXL).
///
/// The cached value is the stage's classified statements, or `None` for a stage
/// with no CXL analysis (Route/Aggregate group keys/Merge) — both outcomes are
/// memoized, so a non-CXL stage is probed once, not once per hop.
struct StageDocCache<'a> {
    config: &'a PipelineConfig,
    by_stage: HashMap<String, Option<Vec<crate::autodoc::CxlStatement>>>,
}

impl<'a> StageDocCache<'a> {
    fn new(config: &'a PipelineConfig) -> Self {
        Self {
            config,
            by_stage: HashMap::new(),
        }
    }

    /// CXL mentions of `field_name` on `stage_id`, parsing-and-caching the stage's
    /// doc on first request (#153). Empty when the stage has no CXL analysis or no
    /// statement touches the field.
    fn mentions(&mut self, stage_id: &str, field_name: &str) -> Vec<CxlMentionView> {
        let config = self.config;
        let statements = self
            .by_stage
            .entry(stage_id.to_string())
            .or_insert_with(|| {
                generate_stage_doc(config, stage_id)
                    .and_then(|doc| doc.cxl_analysis)
                    .map(|analysis| analysis.statements)
            });
        match statements {
            Some(statements) => cxl_mentions_for_field(statements, field_name),
            None => Vec::new(),
        }
    }
}

/// Attach each trace hop's responsible CXL statement(s) by walking the assembled
/// tree and reusing the shared [`StageDocCache`] (#153). Recurses into children so
/// every hop, at any depth, is enriched.
fn enrich_trace_cxl(nodes: &mut [TraceNode], cache: &mut StageDocCache<'_>) {
    for node in nodes.iter_mut() {
        node.cxl_mentions = cache.mentions(&node.endpoint.stage_id, &node.endpoint.field_name);
        enrich_trace_cxl(&mut node.children, cache);
    }
}

/// A `(node_index, field_name)` trace endpoint, borrowing the field name from the
/// view's edges for the duration of one BFS step.
type TraceEndpointKey<'a> = (usize, &'a str);

/// The edge chosen to represent a trace hop: its kind and precision. When several
/// edges reach the same endpoint, the worst-precision one is kept (#148 M2).
type TraceHopEdge = (FieldEdgeKind, Precision);

/// A trace hop discovered by the BFS, recorded flat with a back-reference to the
/// hop it descends from so the spanning tree can be assembled afterward (#153).
/// `parent` is the index into the flat `Vec<TracedHop>` of the discovering hop, or
/// `None` for a hop-1 endpoint discovered directly from the selected root.
struct TracedHop {
    endpoint: TraceEndpointView,
    parent: Option<usize>,
}

/// Walk the field-edge graph from `(start_node, start_field)` and build a
/// hop-by-hop trace TREE (#153). The selected field is the implicit root (hop 0);
/// the returned `Vec<TraceNode>` is its direct (hop-1) children, each carrying its
/// own deeper children.
///
/// The BFS dedups endpoints by `(node, field)` globally, so every reachable
/// endpoint is discovered exactly once — the discovery relation is therefore a
/// spanning TREE, and recording each endpoint's discovering hop preserves the
/// parent→child topology the panel renders. (CXL enrichment is attached later in
/// [`build_field_detail`], where `config` is in scope.)
///
/// `include_indirect` controls the Inspector's INDIRECT include/exclude toggle
/// (#153, the deferred PR3 marker, scoped to this trace): when `false`, edges whose
/// kind is [`EdgeNature::Indirect`](crate::pipeline_view::EdgeNature::Indirect) are
/// skipped while walking, so an influence-only hop (and any subtree reached only
/// through it) never appears. Default callers pass `true` so nothing regresses.
fn trace_tree(
    view: &PipelineView,
    start_node: usize,
    start_field: &str,
    direction: TraceDirection,
    include_indirect: bool,
) -> Vec<TraceNode> {
    let mut seen = HashSet::from([(start_node, start_field.to_string())]);
    // `(node, field, hop, parent_in_hops)` — `parent_in_hops` is the index of the
    // discovering hop in `hops`, or `None` for the selected root.
    let mut queue = VecDeque::from([(start_node, start_field.to_string(), 0usize, None::<usize>)]);
    let mut hops: Vec<TracedHop> = Vec::new();

    while let Some((node, field, hop, parent)) = queue.pop_front() {
        // Collect every edge leaving this anchor, grouped by the endpoint it
        // reaches, keeping the WORST (least-precise) edge per endpoint (#148 M2).
        // The BFS dedups endpoints by `(node, field)` only, and one endpoint can be
        // reached by BOTH an Exact carry and an Approximate INDIRECT influence (same
        // from/to). Picking the worst — rather than whichever edge iterates first —
        // stops an Exact carry from masking a co-incident approximation on the hop
        // badge. Ties on precision keep the first-iterated edge for determinism.
        let mut best_to_endpoint: HashMap<TraceEndpointKey<'_>, TraceHopEdge> = HashMap::new();
        for edge in &view.field_edges {
            // INDIRECT include/exclude toggle (#153): with the toggle off, an
            // influence edge is not traversed, so neither it nor any subtree it
            // uniquely reaches is surfaced. The DIRECT value graph is untouched.
            if !include_indirect && edge.kind.nature() == EdgeNature::Indirect {
                continue;
            }
            let next = match direction {
                TraceDirection::Upstream if edge.to_node == node && edge.to_field == field => {
                    Some((edge.from_node, edge.from_field.as_str()))
                }
                TraceDirection::Downstream
                    if edge.from_node == node && edge.from_field == field =>
                {
                    Some((edge.to_node, edge.to_field.as_str()))
                }
                _ => None,
            };
            let Some((next_node, next_field)) = next else {
                continue;
            };
            best_to_endpoint
                .entry((next_node, next_field))
                .and_modify(|(kind, precision)| {
                    if edge.precision.worst(*precision) != *precision {
                        *kind = edge.kind;
                        *precision = edge.precision;
                    }
                })
                .or_insert((edge.kind, edge.precision));
        }

        // Emit one hop per newly-seen endpoint, sorted for deterministic sibling
        // order (the HashMap iteration order is otherwise nondeterministic). The
        // sort key mirrors the former flat list's tie-break — stage label then
        // field name — and finally the node index, which is unique, so two stages
        // that share a label AND field still order deterministically (a plain
        // label/field key leaves them equal and at the mercy of HashMap order).
        let mut endpoints: Vec<(TraceEndpointKey<'_>, TraceHopEdge)> =
            best_to_endpoint.into_iter().collect();
        endpoints.sort_by(|a, b| {
            let label = |key: &TraceEndpointKey<'_>| {
                view.stages.get(key.0).map(|stage| stage.label.as_str())
            };
            label(&a.0)
                .cmp(&label(&b.0))
                .then_with(|| a.0.1.cmp(b.0.1))
                .then_with(|| a.0.0.cmp(&b.0.0))
        });
        for ((next_node, next_field), (edge_kind, edge_precision)) in endpoints {
            let endpoint = (next_node, next_field.to_string());
            if !seen.insert(endpoint.clone()) {
                continue;
            }
            if let Some(stage) = view.stages.get(next_node) {
                let index = hops.len();
                hops.push(TracedHop {
                    endpoint: trace_endpoint(
                        stage,
                        endpoint.1.clone(),
                        edge_kind,
                        edge_precision,
                        hop + 1,
                    ),
                    parent,
                });
                queue.push_back((next_node, endpoint.1, hop + 1, Some(index)));
            }
        }
    }

    assemble_trace_tree(hops)
}

/// Fold the flat, BFS-ordered hops into a forest of [`TraceNode`]s (#153). Each
/// hop's `parent` is the index of an EARLIER hop (BFS discovers a parent before its
/// children), so a single reverse pass — moving each node into its parent's
/// `children`, or into the root forest when `parent` is `None` — assembles the tree
/// without cloning. Sibling order from the per-level sort is preserved.
fn assemble_trace_tree(hops: Vec<TracedHop>) -> Vec<TraceNode> {
    let mut nodes: Vec<Option<TraceNode>> = hops
        .iter()
        .map(|hop| {
            Some(TraceNode {
                endpoint: hop.endpoint.clone(),
                cxl_mentions: Vec::new(),
                children: Vec::new(),
            })
        })
        .collect();

    let mut roots = Vec::new();
    // Walk high→low so a child (always a higher index than its parent) is already
    // fully built when its parent claims it.
    for index in (0..hops.len()).rev() {
        let node = nodes[index].take().expect("each hop is moved exactly once");
        match hops[index].parent {
            Some(parent) => nodes[parent]
                .as_mut()
                .expect("a parent hop is built before its children are claimed")
                .children
                .push(node),
            None => roots.push(node),
        }
    }
    // The reverse walk pushes siblings in descending index order; restore the
    // ascending (sorted) sibling order at every level.
    reverse_sibling_order(&mut roots);
    roots
}

/// Restore ascending sibling order after [`assemble_trace_tree`]'s reverse walk
/// pushed each level's children in descending discovery order (#153).
fn reverse_sibling_order(nodes: &mut [TraceNode]) {
    nodes.reverse();
    for node in nodes.iter_mut() {
        reverse_sibling_order(&mut node.children);
    }
}

fn trace_endpoint(
    stage: &StageView,
    field_name: String,
    edge_kind: FieldEdgeKind,
    edge_precision: Precision,
    hop: usize,
) -> TraceEndpointView {
    TraceEndpointView {
        stage_id: stage.id.clone(),
        stage_label: stage.label.clone(),
        stage_kind_label: stage.kind.badge_label(),
        stage_kind_attr: stage.kind.kind_attr(),
        field_name,
        edge_kind_label: edge_kind_label(edge_kind),
        edge_kind_attr: edge_kind_attr(edge_kind),
        precision: edge_precision,
        hop,
    }
}

/// Whether `field_name` on stage `stage_index` is part of an Aggregate group-by
/// grain — i.e. it is an endpoint of a `GroupBy` [`FieldEdge`] incident to the
/// stage (#147).
///
/// The grain is represented exactly once, as the INDIRECT `GroupBy` edge (the
/// former `FieldRow::is_aggregate_grain` flag was retired). The group-key output
/// row is the edge's `to` endpoint on the Aggregate stage; the upstream column
/// that drives it is the `from` endpoint — both legitimately wear the badge.
fn is_group_by_grain(view: &PipelineView, stage_index: usize, field_name: &str) -> bool {
    view.field_edges.iter().any(|edge| {
        edge.kind == FieldEdgeKind::GroupBy
            && ((edge.to_node == stage_index && edge.to_field == field_name)
                || (edge.from_node == stage_index && edge.from_field == field_name))
    })
}

fn role_usages(view: &PipelineView, stage_index: usize, field_name: &str) -> Vec<RoleUsageView> {
    let mut usages = view
        .role_edges
        .iter()
        .filter(|edge| edge.from_node == stage_index && edge.from_field == field_name)
        .filter_map(|edge| role_usage(view, edge))
        .collect::<Vec<_>>();
    usages.sort_by(|a, b| {
        a.stage_label
            .cmp(&b.stage_label)
            .then_with(|| a.port_label.cmp(&b.port_label))
    });
    usages
}

fn role_usage(view: &PipelineView, edge: &RoleEdge) -> Option<RoleUsageView> {
    let stage = view.stages.get(edge.to_node)?;
    let port = stage
        .role_ports
        .iter()
        .find(|port| port.id == edge.to_port)?;
    Some(RoleUsageView {
        stage_label: stage.label.clone(),
        stage_kind_label: stage.kind.badge_label(),
        stage_kind_attr: stage.kind.kind_attr(),
        port_label: port.label.clone(),
        role: port.role.clone(),
        edge_kind_label: edge_kind_label(edge.kind),
        edge_kind_attr: edge_kind_attr(edge.kind),
    })
}

fn field_explanation(kind: FieldKind) -> String {
    match kind {
        FieldKind::Declared => {
            "Declared by the selected stage or an input schema; no upstream field transform is implied."
        }
        FieldKind::Emitted => {
            "Emitted by this stage. When CXL statement data is available, reads and writes are listed below."
        }
        FieldKind::PassThrough => {
            "Passed through unchanged from an upstream record unless an access edge shows it was also read."
        }
    }
    .to_string()
}

fn field_kind_label(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Declared => "declared",
        FieldKind::Emitted => "emitted",
        FieldKind::PassThrough => "passthrough",
    }
}

fn field_kind_attr(kind: FieldKind) -> &'static str {
    field_kind_label(kind)
}

fn edge_kind_label(kind: FieldEdgeKind) -> &'static str {
    match kind {
        FieldEdgeKind::Passthrough => "passthrough",
        FieldEdgeKind::Access => "access",
        FieldEdgeKind::Derive => "derive",
        // INDIRECT influence edges (#147).
        FieldEdgeKind::Filter => "filter",
        FieldEdgeKind::GroupBy => "group by",
        FieldEdgeKind::JoinKey => "join key",
        FieldEdgeKind::Conditional => "conditional",
    }
}

/// The `data-kind` attribute slug for an edge kind — the human label
/// ([`edge_kind_label`]) with its two multi-word INDIRECT kinds hyphenated so the
/// attribute is a single token. Deriving from the label (rather than a parallel
/// 7-arm match) keeps the two in lock-step: a new/renamed kind only has to be
/// added to `edge_kind_label`, and only the multi-word kinds are special-cased
/// here.
fn edge_kind_attr(kind: FieldEdgeKind) -> &'static str {
    match kind {
        FieldEdgeKind::GroupBy => "group-by",
        FieldEdgeKind::JoinKey => "join-key",
        _ => edge_kind_label(kind),
    }
}

fn join_or_unavailable(values: &[String]) -> String {
    if values.is_empty() {
        "not available".to_string()
    } else {
        values.join(", ")
    }
}

fn envelope_strategy_name(
    strategy: &clinker_plan::config::pipeline_node::EnvelopeStrategy,
) -> &'static str {
    use clinker_plan::config::pipeline_node::EnvelopeStrategy;
    match strategy {
        EnvelopeStrategy::Preserve => "preserve",
        EnvelopeStrategy::Concat => "concat",
    }
}

fn order_by_summary(order_by: &[clinker_plan::config::SortField]) -> String {
    use clinker_plan::config::SortOrder;
    if order_by.is_empty() {
        return "not configured".to_string();
    }
    order_by
        .iter()
        .map(|field| {
            let direction = match field.order {
                SortOrder::Asc => "asc",
                SortOrder::Desc => "desc",
            };
            format!("{} {direction}", field.field)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn reshape_rule_actions(rule: &clinker_plan::config::pipeline_node::ReshapeRule) -> String {
    let mut parts = Vec::new();
    if rule.mutate.is_some() {
        parts.push("mutate");
    }
    if rule.synthesize.is_some() {
        parts.push("synthesize");
    }
    if parts.is_empty() {
        "trigger-only".to_string()
    } else {
        parts.join(" + ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline_view::{
        FieldEdge, FieldRow, RoleEdge, StageKind, StagePortKind, StagePortRow, derive_pipeline_view,
    };
    use clinker_plan::config::parse_config;

    const VARIANT_PIPELINE: &str = r#"
pipeline:
  name: inspector_variants
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: x, type: int }
  - type: transform
    name: clean
    input: src
    config:
      cxl: |
        emit x2 = x + 1
  - type: aggregate
    name: rollup
    input: clean
    config:
      group_by: [x2]
      cxl: |
        emit count = 1
  - type: route
    name: split
    input: clean
    config:
      conditions:
        high: "x2 > 10"
      default: low
  - type: merge
    name: joined
    inputs: [split.high, split.low]
  - type: combine
    name: combined
    input:
      left: joined
      right: rollup
    config:
      where: "left.x2 == right.x2"
      match: first
      on_miss: skip
      cxl: |
        emit count = right.count
      propagate_ck: driver
  - type: composition
    name: sub
    input: combined
    use: compositions/clean_names.comp.yaml
    config:
      threshold: 10
  - type: reshape
    name: shaped
    input: sub
    config:
      partition_by: [x2]
      rules:
        - name: fill
          when: "x2 > 0"
          synthesize:
            copy_from: trigger
  - type: cull
    name: pruned
    input: shaped
    config:
      partition_by: [x2]
      removed_to: dropped
      rules:
        - name: drop_small
          drop_group_when: "count(*) < 2"
  - type: envelope
    name: framed
    body: pruned
    config:
      strategy: preserve
  - type: output
    name: out
    input: framed
    config:
      name: out
      type: csv
      path: ./out.csv
"#;

    fn build_node(config: &PipelineConfig, view: &PipelineView, name: &str) -> NodeInspectorModel {
        let model = build_selected_inspector(
            InspectorSelection::Node(name.to_string()),
            InspectorBuildContext {
                view,
                config: Some(config),
                channel_mode: ChannelViewMode::Raw,
                compiled_plan_available: false,
                visible_errors: &[],
                schema_warnings: &[],
            },
        );
        match model {
            SelectedInspectorModel::Node(node) => *node,
            other => panic!("expected node model for {name}, got {other:?}"),
        }
    }

    #[test]
    fn node_models_cover_representative_node_kinds() {
        let config = parse_config(VARIANT_PIPELINE).expect("fixture parses");
        let view = derive_pipeline_view(&config);

        let expected = [
            ("src", "SOURCE"),
            ("clean", "TRANSFORM"),
            ("rollup", "AGGREGATE"),
            ("split", "ROUTE"),
            ("joined", "MERGE"),
            ("combined", "COMBINE"),
            ("sub", "COMPOSITION"),
            ("shaped", "RESHAPE"),
            ("pruned", "CULL"),
            ("framed", "ENVELOPE"),
            ("out", "OUTPUT"),
        ];
        for (name, kind) in expected {
            let node = build_node(&config, &view, name);
            assert_eq!(node.kind_label, kind);
            assert!(node.sections.iter().any(|section| section.title == "LOGIC"));
        }

        let route = build_node(&config, &view, "split");
        let logic = route
            .sections
            .iter()
            .find(|section| section.title == "LOGIC")
            .expect("logic section");
        assert!(logic.rows.iter().any(|row| row.label == "branch high"));

        let aggregate = build_node(&config, &view, "rollup");
        assert!(
            aggregate
                .sections
                .iter()
                .any(|section| section.title == "ROLE PORTS" && section.unavailable.is_none())
        );
    }

    /// A structurally-valid pipeline whose transform's `cxl:` block is
    /// syntactically malformed (`emit x =` has no right-hand side). The YAML
    /// parses clean; only edit-time CXL validation can catch the error (#141).
    const MALFORMED_CXL_PIPELINE: &str = r#"
pipeline:
  name: malformed_cxl
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: x, type: int }
  - type: transform
    name: bad
    input: src
    config:
      cxl: |
        emit x =
  - type: output
    name: out
    input: bad
    config:
      name: out
      type: csv
      path: ./out.csv
"#;

    #[test]
    fn malformed_cxl_flips_node_off_ok() {
        let config = parse_config(MALFORMED_CXL_PIPELINE).expect("fixture parses");
        let view = derive_pipeline_view(&config);
        let bad = build_node(&config, &view, "bad");

        let cxl_diag = bad
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.label == "cxl")
            .expect("malformed cxl produces a `cxl` diagnostic");
        assert_eq!(cxl_diag.tone, StatusTone::Error);
        assert!(!cxl_diag.message.is_empty());

        assert!(
            bad.status_chips
                .iter()
                .any(|chip| chip.label == "errors" && chip.tone == StatusTone::Error),
            "malformed cxl should yield an `errors` chip, got {:?}",
            bad.status_chips
        );
        assert!(
            !bad.status_chips.iter().any(|chip| chip.label == "ok"),
            "malformed cxl must not report `ok`, got {:?}",
            bad.status_chips
        );
    }

    #[test]
    fn valid_cxl_keeps_node_ok() {
        let config = parse_config(VARIANT_PIPELINE).expect("fixture parses");
        let view = derive_pipeline_view(&config);
        // `clean` carries `emit x2 = x + 1`, which is well-formed CXL.
        let clean = build_node(&config, &view, "clean");

        assert!(
            !clean
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.label == "cxl"),
            "valid cxl should not produce a `cxl` diagnostic, got {:?}",
            clean.diagnostics
        );
        assert!(
            clean
                .status_chips
                .iter()
                .any(|chip| chip.label == "ok" && chip.tone == StatusTone::Ok),
            "valid cxl should yield an `ok` chip, got {:?}",
            clean.status_chips
        );
    }

    fn stage(id: &str, kind: StageKind, fields: Vec<FieldRow>) -> StageView {
        StageView {
            id: id.to_string(),
            label: id.to_string(),
            kind,
            subtitle: String::new(),
            canvas_x: 0.0,
            canvas_y: 0.0,
            cxl_source: None,
            description: None,
            error_message: None,
            fields,
            branches: Vec::new(),
            role_ports: Vec::new(),
        }
    }

    #[test]
    fn field_model_enriches_declared_emitted_passthrough_and_role_use() {
        let source = stage(
            "src",
            StageKind::Source,
            vec![FieldRow {
                name: "x".to_string(),
                kind: FieldKind::Declared,
                ty: Some("int".to_string()),
                is_correlation_key: true,
                ..Default::default()
            }],
        );
        let transform = stage(
            "clean",
            StageKind::Transform,
            vec![
                FieldRow {
                    name: "x".to_string(),
                    kind: FieldKind::PassThrough,
                    ty: Some("int".to_string()),
                    ..Default::default()
                },
                FieldRow {
                    name: "x2".to_string(),
                    kind: FieldKind::Emitted,
                    ..Default::default()
                },
            ],
        );
        let mut aggregate = stage(
            "rollup",
            StageKind::Aggregate,
            vec![FieldRow {
                name: "x2".to_string(),
                kind: FieldKind::Declared,
                ..Default::default()
            }],
        );
        aggregate.role_ports = vec![StagePortRow {
            id: "group_by:x2".to_string(),
            label: "x2".to_string(),
            role: "group_by".to_string(),
            kind: StagePortKind::AggregateGroupKey,
            side: StagePortSide::Input,
        }];
        let view = PipelineView {
            stages: vec![source, transform, aggregate],
            field_edges: vec![
                FieldEdge {
                    from_node: 0,
                    from_field: "x".to_string(),
                    to_node: 1,
                    to_field: "x".to_string(),
                    kind: FieldEdgeKind::Passthrough,
                    ..Default::default()
                },
                FieldEdge {
                    from_node: 0,
                    from_field: "x".to_string(),
                    to_node: 1,
                    to_field: "x2".to_string(),
                    kind: FieldEdgeKind::Derive,
                    ..Default::default()
                },
            ],
            role_edges: vec![RoleEdge {
                from_node: 1,
                from_field: "x2".to_string(),
                to_node: 2,
                to_port: "group_by:x2".to_string(),
                kind: FieldEdgeKind::Access,
            }],
            ..Default::default()
        };

        let emitted = build_field_detail(&view, None, &SelectedField::new("clean", "x2"))
            .expect("field exists");
        assert_eq!(emitted.field_kind_label, "emitted");
        // The upstream trace is a TREE; `clean.x2` has one direct (hop-1) parent,
        // `src.x` via the Derive edge, with no deeper hops.
        assert_eq!(emitted.upstream.len(), 1);
        assert!(emitted.upstream[0].children.is_empty());
        assert_eq!(emitted.upstream[0].endpoint.hop, 1);
        assert_eq!(emitted.role_usages.len(), 1);

        let declared =
            build_field_detail(&view, None, &SelectedField::new("src", "x")).expect("field exists");
        assert!(declared.badges.contains(&"correlation key".to_string()));

        let passthrough = build_field_detail(&view, None, &SelectedField::new("clean", "x"))
            .expect("field exists");
        assert_eq!(passthrough.field_kind_label, "passthrough");

        // #151: an upstream trace hop round-trips to a selectable canvas field —
        // its `to_selected_field()` carries the exact (stage_id, field_name)
        // identity, which `build_field_detail` (what the inspector rebuilds from
        // on selection) resolves back to that same field.
        let hop = &emitted.upstream[0].endpoint;
        let hop_selection = hop.to_selected_field();
        assert_eq!(hop_selection.stage_id, hop.stage_id);
        assert_eq!(hop_selection.field_name, hop.field_name);
        let resolved = build_field_detail(&view, None, &hop_selection)
            .expect("trace hop resolves to a canvas field");
        assert_eq!(resolved.selection, hop_selection);
    }

    /// The aggregate-grain badge (#147) is derived from an incident `GroupBy`
    /// edge — the grain is represented exactly once (the edge), not a row flag.
    /// Both endpoints of the GroupBy edge wear the badge; an unrelated field on
    /// the same stage does not.
    #[test]
    fn aggregate_grain_badge_comes_from_group_by_edge() {
        let source = stage(
            "src",
            StageKind::Source,
            vec![FieldRow {
                name: "region".to_string(),
                kind: FieldKind::Declared,
                ..Default::default()
            }],
        );
        let aggregate = stage(
            "rollup",
            StageKind::Aggregate,
            vec![
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
            ],
        );
        let view = PipelineView {
            stages: vec![source, aggregate],
            field_edges: vec![FieldEdge {
                from_node: 0,
                from_field: "region".to_string(),
                to_node: 1,
                to_field: "region".to_string(),
                kind: FieldEdgeKind::GroupBy,
                ..Default::default()
            }],
            ..Default::default()
        };

        let group_key = build_field_detail(&view, None, &SelectedField::new("rollup", "region"))
            .expect("field exists");
        assert!(
            group_key.badges.contains(&"aggregate grain".to_string()),
            "the GroupBy edge target row wears the grain badge"
        );

        let upstream_driver = build_field_detail(&view, None, &SelectedField::new("src", "region"))
            .expect("field exists");
        assert!(
            upstream_driver
                .badges
                .contains(&"aggregate grain".to_string()),
            "the GroupBy edge source column also drives the grain"
        );

        let aggregate_value =
            build_field_detail(&view, None, &SelectedField::new("rollup", "total"))
                .expect("field exists");
        assert!(
            !aggregate_value
                .badges
                .contains(&"aggregate grain".to_string()),
            "an aggregate value column is not part of the grain"
        );
    }

    #[test]
    fn field_model_reports_no_lineage_and_missing_current_view() {
        let view = PipelineView {
            stages: vec![stage(
                "src",
                StageKind::Source,
                vec![FieldRow {
                    name: "lonely".to_string(),
                    kind: FieldKind::Declared,
                    ..Default::default()
                }],
            )],
            ..Default::default()
        };

        let detail = build_field_detail(&view, None, &SelectedField::new("src", "lonely"))
            .expect("field exists");
        // #148: the empty-state is now surfaced through the precision fields, but
        // the original "no field-level lineage edges" message MUST be preserved
        // verbatim (acceptance forbids regressing it) and the field flagged empty.
        assert!(
            detail.lineage_empty,
            "an edgeless field must be flagged empty"
        );
        assert_eq!(
            detail.precision_reason,
            "No field-level lineage edges mention this field in the current view.",
            "the original empty-state message must be preserved verbatim"
        );
        assert!(build_field_detail(&view, None, &SelectedField::new("missing", "x")).is_none());

        // #151: a trace hop pointing at a field absent from the current view
        // resolves to no detail — selecting it surfaces the Missing inspector
        // rather than stale content.
        let stale_hop = TraceEndpointView {
            stage_id: "missing".to_string(),
            stage_label: "Missing".to_string(),
            stage_kind_label: "Source",
            stage_kind_attr: "source",
            field_name: "x".to_string(),
            edge_kind_label: "derive",
            edge_kind_attr: "derive",
            precision: Precision::Exact,
            hop: 1,
        };
        assert!(build_field_detail(&view, None, &stale_hop.to_selected_field()).is_none());
    }

    /// #148: a field whose row precision is Approximate surfaces that tier on the
    /// inspector model, AND each trace hop carries the precision of the edge taken
    /// to reach it. The Approximate INDIRECT (Filter) hop reads `approximate`; the
    /// Exact carry hop reads `exact`. A genuine break would mislabel a hop's tier.
    #[test]
    fn field_precision_and_per_hop_precision_surface() {
        // src.flag --Filter(Approximate)--> kept.kept  (INDIRECT influence)
        // src.kept --Passthrough(Exact)---> kept.kept  (DIRECT carry)
        // The downstream `kept.kept` row is built Approximate (its producing Filter
        // edge degrades it); selecting it shows an upstream Filter hop reading
        // approximate and an upstream passthrough hop reading exact.
        let source = stage(
            "src",
            StageKind::Source,
            vec![
                FieldRow {
                    name: "flag".to_string(),
                    kind: FieldKind::Declared,
                    ..Default::default()
                },
                FieldRow {
                    name: "kept".to_string(),
                    kind: FieldKind::Declared,
                    ..Default::default()
                },
            ],
        );
        let cull = stage(
            "keep",
            StageKind::Cull,
            vec![FieldRow {
                name: "kept".to_string(),
                kind: FieldKind::PassThrough,
                // Row built Approximate by its producing Filter edge (#148).
                lineage_precision: Precision::Approximate,
                precision_reason: "INDIRECT filter predicate influence",
                ..Default::default()
            }],
        );
        let view = PipelineView {
            stages: vec![source, cull],
            field_edges: vec![
                FieldEdge::influence(0, "flag".into(), 1, "kept".into(), FieldEdgeKind::Filter),
                FieldEdge::carry(
                    0,
                    "kept".into(),
                    1,
                    "kept".into(),
                    FieldEdgeKind::Passthrough,
                ),
            ],
            ..Default::default()
        };

        let detail = build_field_detail(&view, None, &SelectedField::new("keep", "kept"))
            .expect("field exists");
        assert!(!detail.lineage_empty, "an edged field is not empty");
        assert_eq!(
            detail.lineage_precision,
            Precision::Approximate,
            "the field's precision reflects its Approximate row + Filter hop"
        );

        // Per-hop precision (#148 M2 carries the enum): the two upstream edges land
        // on DISTINCT endpoints (`src.flag` via Filter, `src.kept` via Passthrough),
        // so both hops emit as hop-1 children — the Filter hop is Approximate, the
        // carry hop Exact.
        let filter_hop = detail
            .upstream
            .iter()
            .map(|node| &node.endpoint)
            .find(|hop| hop.edge_kind_attr == "filter")
            .expect("a Filter upstream hop");
        assert_eq!(filter_hop.precision, Precision::Approximate);
        let carry_hop = detail
            .upstream
            .iter()
            .map(|node| &node.endpoint)
            .find(|hop| hop.edge_kind_attr == "passthrough")
            .expect("a passthrough upstream hop");
        assert_eq!(carry_hop.precision, Precision::Exact);
    }

    /// #148: an Exact field with lineage edges is NOT flagged empty and reports
    /// `exact` — distinguishing it from the edgeless empty-state, which keeps the
    /// preserved "no lineage edges" message (covered by
    /// `field_model_reports_no_lineage_and_missing_current_view`).
    #[test]
    fn exact_field_with_edges_is_not_empty() {
        let source = stage(
            "src",
            StageKind::Source,
            vec![FieldRow {
                name: "a".to_string(),
                kind: FieldKind::Declared,
                ..Default::default()
            }],
        );
        let derived = stage(
            "calc",
            StageKind::Transform,
            vec![FieldRow {
                name: "y".to_string(),
                kind: FieldKind::Emitted,
                ..Default::default()
            }],
        );
        let view = PipelineView {
            stages: vec![source, derived],
            field_edges: vec![FieldEdge::derive(0, "a".into(), 1, "y".into(), false)],
            ..Default::default()
        };
        let detail = build_field_detail(&view, None, &SelectedField::new("calc", "y"))
            .expect("field exists");
        assert!(!detail.lineage_empty);
        assert_eq!(detail.lineage_precision, Precision::Exact);
    }

    /// #148 M1: the field-level badge reflects the field's OWN provenance, NOT a
    /// transitive trace fold. A PRISTINE source column that merely FEEDS a
    /// downstream Cull reads Exact (matching the canvas node-corner, which reads the
    /// same `FieldRow::lineage_precision`) — even though its downstream hop is an
    /// Approximate Filter influence. The DOWNSTREAM hop badge still shows
    /// Approximate, so the approximation is visible where it occurs without
    /// painting the upstream source Approximate.
    #[test]
    fn field_badge_is_rows_own_precision_not_a_trace_fold() {
        // src.flag (a clean Exact source row) --Filter--> keep.flag (downstream Cull
        // row, degraded to Approximate by its producing Filter edge).
        let source = stage(
            "src",
            StageKind::Source,
            vec![FieldRow {
                name: "flag".to_string(),
                kind: FieldKind::Declared,
                // A pristine source row: Exact, no degraded producing edge.
                ..Default::default()
            }],
        );
        let cull = stage(
            "keep",
            StageKind::Cull,
            vec![FieldRow {
                name: "flag".to_string(),
                kind: FieldKind::PassThrough,
                lineage_precision: Precision::Approximate,
                precision_reason: "INDIRECT filter predicate influence",
                ..Default::default()
            }],
        );
        let view = PipelineView {
            stages: vec![source, cull],
            field_edges: vec![FieldEdge::influence(
                0,
                "flag".into(),
                1,
                "flag".into(),
                FieldEdgeKind::Filter,
            )],
            ..Default::default()
        };

        // The PRISTINE source field reads Exact (its own row precision), NOT
        // Approximate — it is not dragged down by the downstream Filter hop.
        let src_detail = build_field_detail(&view, None, &SelectedField::new("src", "flag"))
            .expect("source field exists");
        assert_eq!(
            src_detail.lineage_precision,
            Precision::Exact,
            "a pristine source feeding a downstream Cull must stay Exact (matches the node-corner)"
        );
        // Its downstream hop still surfaces the Approximate Filter influence.
        let down_hop = src_detail
            .downstream
            .iter()
            .map(|node| &node.endpoint)
            .find(|hop| hop.edge_kind_attr == "filter")
            .expect("a downstream Filter hop");
        assert_eq!(
            down_hop.precision,
            Precision::Approximate,
            "the approximation is shown on the hop, not folded onto the source field"
        );
    }

    /// #148 M2: when a trace endpoint is reachable by BOTH an Exact carry and an
    /// Approximate INDIRECT edge (same from/to), the single emitted hop surfaces the
    /// WORST (least-precise) edge — the Exact carry must not mask the approximation.
    #[test]
    fn colliding_hop_surfaces_worst_precision() {
        // src.k reaches keep.k by BOTH a Passthrough carry (Exact) AND a JoinKey
        // influence (Approximate) — the value carry and its influence overlay
        // coexist on one (from, to) endpoint.
        let source = stage(
            "src",
            StageKind::Source,
            vec![FieldRow {
                name: "k".to_string(),
                kind: FieldKind::Declared,
                ..Default::default()
            }],
        );
        let join = stage(
            "j",
            StageKind::Combine,
            vec![FieldRow {
                name: "k".to_string(),
                kind: FieldKind::PassThrough,
                ..Default::default()
            }],
        );
        let view = PipelineView {
            stages: vec![source, join],
            field_edges: vec![
                // Exact carry pushed FIRST — under naive first-wins dedup it would
                // have masked the approximation.
                FieldEdge::carry(0, "k".into(), 1, "k".into(), FieldEdgeKind::Passthrough),
                FieldEdge::influence(0, "k".into(), 1, "k".into(), FieldEdgeKind::JoinKey),
            ],
            ..Default::default()
        };

        let detail =
            build_field_detail(&view, None, &SelectedField::new("j", "k")).expect("field exists");
        // Exactly ONE upstream hop to (src, k) — the endpoint is deduped — and it
        // surfaces the WORST precision (Approximate), not the first-iterated Exact.
        let hops: Vec<_> = detail
            .upstream
            .iter()
            .map(|node| &node.endpoint)
            .filter(|h| h.field_name == "k")
            .collect();
        assert_eq!(hops.len(), 1, "the colliding endpoint dedups to one hop");
        assert_eq!(
            hops[0].precision,
            Precision::Approximate,
            "the hop must surface the worst (Approximate) edge, not the first-iterated Exact carry"
        );
    }

    /// #153: a multi-hop upstream trace renders as a TREE — a hop-2 endpoint is a
    /// CHILD of the hop-1 endpoint it was discovered from, NOT a sibling at the
    /// root. A flat sorted list would have lost this parent→child topology. Chain:
    /// `src.x --Derive--> mid.y --Derive--> sink.z`; selecting `sink.z` upstream
    /// yields one hop-1 child (`mid.y`) whose only child is hop-2 (`src.x`).
    #[test]
    fn multi_hop_upstream_trace_preserves_parent_child_topology() {
        let source = stage(
            "src",
            StageKind::Source,
            vec![FieldRow {
                name: "x".to_string(),
                kind: FieldKind::Declared,
                ..Default::default()
            }],
        );
        let mid = stage(
            "mid",
            StageKind::Transform,
            vec![FieldRow {
                name: "y".to_string(),
                kind: FieldKind::Emitted,
                ..Default::default()
            }],
        );
        let sink = stage(
            "sink",
            StageKind::Transform,
            vec![FieldRow {
                name: "z".to_string(),
                kind: FieldKind::Emitted,
                ..Default::default()
            }],
        );
        let view = PipelineView {
            stages: vec![source, mid, sink],
            field_edges: vec![
                FieldEdge::derive(0, "x".into(), 1, "y".into(), false),
                FieldEdge::derive(1, "y".into(), 2, "z".into(), false),
            ],
            ..Default::default()
        };

        let detail = build_field_detail(&view, None, &SelectedField::new("sink", "z"))
            .expect("field exists");

        // Hop-1: exactly one direct parent, `mid.y`, at the root of the forest.
        assert_eq!(detail.upstream.len(), 1, "one direct (hop-1) parent");
        let hop1 = &detail.upstream[0];
        assert_eq!(hop1.endpoint.stage_id, "mid");
        assert_eq!(hop1.endpoint.field_name, "y");
        assert_eq!(hop1.endpoint.hop, 1);

        // Hop-2 (`src.x`) is a CHILD of hop-1, NOT a second root sibling.
        assert_eq!(
            detail.upstream.len(),
            1,
            "the hop-2 endpoint must not appear as a root sibling"
        );
        assert_eq!(hop1.children.len(), 1, "hop-1 has exactly one deeper hop");
        let hop2 = &hop1.children[0];
        assert_eq!(hop2.endpoint.stage_id, "src");
        assert_eq!(hop2.endpoint.field_name, "x");
        assert_eq!(hop2.endpoint.hop, 2);
        assert!(
            hop2.children.is_empty(),
            "the chain terminates at the source"
        );

        // The summary counts EVERY traced node, not just the direct hops.
        assert_eq!(count_trace_nodes(&detail.upstream), 2);
    }

    /// #153: a BRANCHING upstream trace keeps each deeper hop under the CORRECT
    /// hop-1 parent, with siblings in deterministic (stage-label, field-name) order.
    /// Two hop-1 parents (`midA.a`, `midB.b`) each derive `sink.z`; `midA.a` is in
    /// turn derived from TWO sources (`srcP.p`, `srcQ.q`). A flat list would lose
    /// which parent each hop-2 descends from.
    #[test]
    fn branching_trace_groups_children_under_their_own_parent() {
        let stages = vec![
            stage(
                "srcP",
                StageKind::Source,
                vec![FieldRow {
                    name: "p".to_string(),
                    kind: FieldKind::Declared,
                    ..Default::default()
                }],
            ),
            stage(
                "srcQ",
                StageKind::Source,
                vec![FieldRow {
                    name: "q".to_string(),
                    kind: FieldKind::Declared,
                    ..Default::default()
                }],
            ),
            stage(
                "midA",
                StageKind::Transform,
                vec![FieldRow {
                    name: "a".to_string(),
                    kind: FieldKind::Emitted,
                    ..Default::default()
                }],
            ),
            stage(
                "midB",
                StageKind::Transform,
                vec![FieldRow {
                    name: "b".to_string(),
                    kind: FieldKind::Declared,
                    ..Default::default()
                }],
            ),
            stage(
                "sink",
                StageKind::Transform,
                vec![FieldRow {
                    name: "z".to_string(),
                    kind: FieldKind::Emitted,
                    ..Default::default()
                }],
            ),
        ];
        let view = PipelineView {
            stages,
            field_edges: vec![
                // sink.z derived from BOTH midA.a and midB.b (two hop-1 parents).
                FieldEdge::derive(2, "a".into(), 4, "z".into(), false),
                FieldEdge::derive(3, "b".into(), 4, "z".into(), false),
                // midA.a derived from TWO sources (two hop-2 children of midA).
                FieldEdge::derive(0, "p".into(), 2, "a".into(), false),
                FieldEdge::derive(1, "q".into(), 2, "a".into(), false),
            ],
            ..Default::default()
        };

        let detail = build_field_detail(&view, None, &SelectedField::new("sink", "z"))
            .expect("field exists");

        // Two hop-1 parents in (stage-label) order: midA before midB.
        assert_eq!(detail.upstream.len(), 2);
        assert_eq!(detail.upstream[0].endpoint.stage_id, "midA");
        assert_eq!(detail.upstream[1].endpoint.stage_id, "midB");

        // midA's TWO hop-2 children land under midA (NOT midB), sorted srcP then srcQ.
        let mid_a = &detail.upstream[0];
        assert_eq!(mid_a.children.len(), 2, "midA has both source hops");
        assert_eq!(mid_a.children[0].endpoint.stage_id, "srcP");
        assert_eq!(mid_a.children[1].endpoint.stage_id, "srcQ");
        for child in &mid_a.children {
            assert_eq!(child.endpoint.hop, 2);
        }

        // midB has no deeper hop (its `b` is a declared root).
        assert!(detail.upstream[1].children.is_empty());

        // Every endpoint reached exactly once: 2 hop-1 + 2 hop-2 = 4 nodes.
        assert_eq!(count_trace_nodes(&detail.upstream), 4);
    }

    /// #153: each hop names its transform — the edge-kind label and per-hop
    /// precision are carried on every node — AND a hop on a CXL stage attaches the
    /// responsible statement(s), while a hop on a non-CXL stage (here a Source)
    /// attaches none. Built from a real config so `generate_stage_doc` runs.
    #[test]
    fn each_hop_names_transform_and_attaches_cxl_only_for_cxl_stages() {
        // src(source, no CXL) -> clean(transform, `emit y = x + 1`).
        // Selecting `clean.y` upstream yields one hop-1 node at `src.x`.
        const PIPELINE: &str = r#"
pipeline:
  name: hop_attribution
nodes:
  - type: source
    name: src
    config:
      name: src
      type: csv
      path: ./in.csv
      schema:
        - { name: x, type: int }
  - type: transform
    name: clean
    input: src
    config:
      cxl: |
        emit y = x + 1
  - type: output
    name: out
    input: clean
    config:
      name: out
      type: csv
      path: ./out.csv
"#;
        let config = parse_config(PIPELINE).expect("fixture parses");
        let view = derive_pipeline_view(&config);

        // Downstream of `src.x`: a hop lands on the DERIVED `clean.y`, a CXL stage,
        // so it carries the `emit y = x + 1` statement that produced it.
        let src_detail = build_field_detail(&view, Some(&config), &SelectedField::new("src", "x"))
            .expect("source field exists");
        let derived_hop = src_detail
            .downstream
            .iter()
            .find(|node| node.endpoint.stage_id == "clean" && node.endpoint.field_name == "y")
            .expect("a downstream hop onto the derived transform field");
        // Names its transform: edge kind + precision are present per hop.
        assert_eq!(derived_hop.endpoint.edge_kind_label, "derive");
        assert_eq!(derived_hop.endpoint.precision, Precision::Exact);
        // CXL attribution present for the transform hop.
        assert!(
            derived_hop
                .cxl_mentions
                .iter()
                .any(|m| m.expression.contains("x + 1")),
            "the transform hop attaches its producing CXL statement, got {:?}",
            derived_hop.cxl_mentions
        );

        // Upstream of `clean.y`: the hop-1 node lands on `src.x`, a Source — no CXL
        // analysis — so it attaches NO statement; the edge kind/precision is the
        // attribution there.
        let clean_detail =
            build_field_detail(&view, Some(&config), &SelectedField::new("clean", "y"))
                .expect("transform field exists");
        let src_hop = clean_detail
            .upstream
            .iter()
            .find(|node| node.endpoint.stage_id == "src")
            .expect("an upstream hop onto the source");
        assert!(
            src_hop.cxl_mentions.is_empty(),
            "a non-CXL Source hop attaches no statement, got {:?}",
            src_hop.cxl_mentions
        );
    }

    /// #153: the Inspector's INDIRECT include/exclude toggle. The full tree
    /// (`upstream`, toggle ON / default) carries the INDIRECT influence hop; the
    /// direct-only tree (`upstream_direct`, toggle OFF) excludes that hop AND any
    /// subtree reachable only through it, while leaving the DIRECT value graph
    /// intact. Fixture: `src.flag --Filter--> keep.kept` (INDIRECT) coexisting with
    /// `src.kept --Passthrough--> keep.kept` (DIRECT carry).
    #[test]
    fn indirect_toggle_prunes_influence_hops_from_built_tree() {
        let source = stage(
            "src",
            StageKind::Source,
            vec![
                FieldRow {
                    name: "flag".to_string(),
                    kind: FieldKind::Declared,
                    ..Default::default()
                },
                FieldRow {
                    name: "kept".to_string(),
                    kind: FieldKind::Declared,
                    ..Default::default()
                },
            ],
        );
        let keep = stage(
            "keep",
            StageKind::Cull,
            vec![FieldRow {
                name: "kept".to_string(),
                kind: FieldKind::PassThrough,
                ..Default::default()
            }],
        );
        let view = PipelineView {
            stages: vec![source, keep],
            field_edges: vec![
                FieldEdge::influence(0, "flag".into(), 1, "kept".into(), FieldEdgeKind::Filter),
                FieldEdge::carry(
                    0,
                    "kept".into(),
                    1,
                    "kept".into(),
                    FieldEdgeKind::Passthrough,
                ),
            ],
            ..Default::default()
        };

        let detail = build_field_detail(&view, None, &SelectedField::new("keep", "kept"))
            .expect("field exists");

        // Default (toggle ON): BOTH hops are present — the DIRECT carry to `src.kept`
        // AND the INDIRECT Filter influence onto `src.flag`.
        let kinds: Vec<_> = detail
            .upstream
            .iter()
            .map(|node| node.endpoint.edge_kind_attr)
            .collect();
        assert!(
            kinds.contains(&"filter"),
            "the INDIRECT Filter hop is included by default, got {kinds:?}"
        );
        assert!(
            kinds.contains(&"passthrough"),
            "the DIRECT carry hop is always present, got {kinds:?}"
        );

        // Toggle OFF (the direct-only tree): the Filter hop (and anything reached
        // only through it) is excluded; the DIRECT carry survives.
        let direct_kinds: Vec<_> = detail
            .upstream_direct
            .iter()
            .map(|node| node.endpoint.edge_kind_attr)
            .collect();
        assert!(
            !direct_kinds.contains(&"filter"),
            "the INDIRECT Filter hop must be excluded when the toggle is off, got {direct_kinds:?}"
        );
        assert!(
            direct_kinds.contains(&"passthrough"),
            "the DIRECT carry hop must survive in the direct-only tree, got {direct_kinds:?}"
        );
    }

    /// #153 regression: two distinct sibling hops that share a stage label AND a
    /// field name must still order deterministically. The per-level sort tie-breaks
    /// on the unique node index after (label, field), so HashMap iteration order
    /// cannot leak through and make the rendered sibling order (and the
    /// default-expanded set) flip run-to-run.
    #[test]
    fn same_label_same_field_siblings_order_deterministically() {
        let mut sink = stage(
            "sink",
            StageKind::Merge,
            vec![FieldRow {
                name: "v".to_string(),
                kind: FieldKind::PassThrough,
                ..Default::default()
            }],
        );
        sink.label = "clean".to_string();
        let mut p0 = stage(
            "p0",
            StageKind::Source,
            vec![FieldRow {
                name: "x".to_string(),
                kind: FieldKind::Declared,
                ..Default::default()
            }],
        );
        // Two producers sharing BOTH the display label and the field name; only their
        // (unique) node index distinguishes them.
        p0.label = "clean".to_string();
        let mut p1 = stage(
            "p1",
            StageKind::Source,
            vec![FieldRow {
                name: "x".to_string(),
                kind: FieldKind::Declared,
                ..Default::default()
            }],
        );
        p1.label = "clean".to_string();
        let view = PipelineView {
            stages: vec![sink, p0, p1],
            field_edges: vec![
                FieldEdge::carry(1, "x".into(), 0, "v".into(), FieldEdgeKind::Passthrough),
                FieldEdge::carry(2, "x".into(), 0, "v".into(), FieldEdgeKind::Passthrough),
            ],
            ..Default::default()
        };

        let detail = build_field_detail(&view, None, &SelectedField::new("sink", "v"))
            .expect("field exists");
        let order: Vec<_> = detail
            .upstream
            .iter()
            .map(|node| node.endpoint.stage_id.as_str())
            .collect();
        assert_eq!(
            order,
            vec!["p0", "p1"],
            "same-label/same-field siblings order by node index, not HashMap order"
        );
    }

    /// #153 regression: an endpoint reached by BOTH a DIRECT carry and an INDIRECT
    /// influence (a dual-role column, e.g. a Combine join key that is also carried as
    /// a value) must remain visible — correctly tagged DIRECT — when the INDIRECT
    /// toggle is off. The full tree's worst-precision dedup tags the merged hop
    /// INDIRECT (Approximate masks the Exact carry on the badge, per #148); a naive
    /// prune of that built tree would then drop the column entirely. Building the
    /// direct-only tree with `include_indirect = false` instead walks the surviving
    /// carry edge, so the value hop is kept and re-tagged DIRECT.
    #[test]
    fn dual_role_endpoint_survives_direct_only_tree_as_direct() {
        let source = stage(
            "src",
            StageKind::Source,
            vec![FieldRow {
                name: "k".to_string(),
                kind: FieldKind::Declared,
                ..Default::default()
            }],
        );
        let out = stage(
            "out",
            StageKind::Combine,
            vec![FieldRow {
                name: "k".to_string(),
                kind: FieldKind::PassThrough,
                ..Default::default()
            }],
        );
        // The SAME endpoint `src.k -> out.k` carries a value (Passthrough, DIRECT)
        // and drives the join (JoinKey, INDIRECT).
        let view = PipelineView {
            stages: vec![source, out],
            field_edges: vec![
                FieldEdge::influence(0, "k".into(), 1, "k".into(), FieldEdgeKind::JoinKey),
                FieldEdge::carry(0, "k".into(), 1, "k".into(), FieldEdgeKind::Passthrough),
            ],
            ..Default::default()
        };

        let detail =
            build_field_detail(&view, None, &SelectedField::new("out", "k")).expect("field exists");

        // Full tree: the merged hop is tagged with the worst (INDIRECT JoinKey) edge.
        assert_eq!(
            detail
                .upstream
                .iter()
                .map(|node| node.endpoint.edge_kind_attr)
                .collect::<Vec<_>>(),
            vec!["join-key"],
            "the worst-precision dedup tags the dual-role hop INDIRECT in the full tree"
        );

        // Direct-only tree: the value hop survives via its carry edge, tagged DIRECT —
        // it is NOT dropped just because an influence edge also reaches it.
        assert_eq!(
            detail
                .upstream_direct
                .iter()
                .map(|node| node.endpoint.edge_kind_attr)
                .collect::<Vec<_>>(),
            vec!["passthrough"],
            "a dual-role column keeps its DIRECT carry hop when INDIRECT is off"
        );
    }

    /// `edge_kind_attr` feeds the `data-kind` HTML attribute, so EVERY kind's
    /// value must be a single slug token — no whitespace. The attr derives from
    /// `edge_kind_label` (which has multi-word labels like "group by"/"join key"),
    /// hyphenating only the two known multi-word kinds; this guards against a
    /// future multi-word kind leaking a space through that delegation. Every
    /// variant is listed by name (no wildcard) so adding a kind without a slug
    /// decision fails to compile.
    #[test]
    fn edge_kind_attr_is_always_slug_safe() {
        for kind in [
            FieldEdgeKind::Passthrough,
            FieldEdgeKind::Access,
            FieldEdgeKind::Derive,
            FieldEdgeKind::Filter,
            FieldEdgeKind::GroupBy,
            FieldEdgeKind::JoinKey,
            FieldEdgeKind::Conditional,
        ] {
            let attr = edge_kind_attr(kind);
            assert!(
                !attr.is_empty(),
                "{kind:?} must have a non-empty data-kind slug"
            );
            assert!(
                !attr.chars().any(char::is_whitespace),
                "{kind:?} data-kind slug must contain no whitespace, got {attr:?}"
            );
        }
    }
}
