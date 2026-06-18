use std::collections::{HashSet, VecDeque};

use clinker_plan::config::{PipelineConfig, PipelineNode};

use crate::autodoc::{CxlStatementKind, generate_stage_doc};
use crate::notes::parse_notes;
use crate::pipeline_view::{
    FieldEdgeKind, FieldKind, PipelineView, RoleEdge, StagePortSide, StageView,
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
    pub upstream: Vec<TraceEndpointView>,
    pub downstream: Vec<TraceEndpointView>,
    pub role_usages: Vec<RoleUsageView>,
    pub lineage_unavailable_reason: Option<String>,
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

    let diagnostics = node_diagnostics(stage, ctx.visible_errors, ctx.schema_warnings);
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
        cxl_section(stage_id, node, doc.as_ref()),
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

fn node_diagnostics(
    stage: Option<&StageView>,
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
) -> InspectorSection {
    let Some(cxl_source) = node.and_then(node_cxl_source) else {
        return InspectorSection::unavailable("CXL", "This node has no top-level CXL block.");
    };
    let mut rows = Vec::new();
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

    let upstream = trace_endpoints(
        view,
        stage_index,
        &selection.field_name,
        TraceDirection::Upstream,
    );
    let downstream = trace_endpoints(
        view,
        stage_index,
        &selection.field_name,
        TraceDirection::Downstream,
    );
    let role_usages = role_usages(view, stage_index, &selection.field_name);
    let mut badges = Vec::new();
    if field.is_correlation_key {
        badges.push("correlation key".to_string());
    }
    if field.is_aggregate_grain {
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

    let lineage_unavailable_reason =
        if upstream.is_empty() && downstream.is_empty() && role_usages.is_empty() {
            Some("No field-level lineage edges mention this field in the current view.".to_string())
        } else {
            None
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
                    upstream.len(),
                    downstream.len(),
                    role_usages.len()
                ),
            ),
        ],
        explanation: field_explanation(field.kind),
        annotation,
        cxl_mentions,
        upstream,
        downstream,
        role_usages,
        lineage_unavailable_reason,
    })
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

fn trace_endpoints(
    view: &PipelineView,
    start_node: usize,
    start_field: &str,
    direction: TraceDirection,
) -> Vec<TraceEndpointView> {
    let mut seen = HashSet::from([(start_node, start_field.to_string())]);
    let mut queue = VecDeque::from([(start_node, start_field.to_string(), 0usize)]);
    let mut out = Vec::new();

    while let Some((node, field, hop)) = queue.pop_front() {
        for edge in &view.field_edges {
            let next = match direction {
                TraceDirection::Upstream if edge.to_node == node && edge.to_field == field => {
                    Some((edge.from_node, edge.from_field.as_str(), edge.kind))
                }
                TraceDirection::Downstream
                    if edge.from_node == node && edge.from_field == field =>
                {
                    Some((edge.to_node, edge.to_field.as_str(), edge.kind))
                }
                _ => None,
            };

            let Some((next_node, next_field, edge_kind)) = next else {
                continue;
            };
            let endpoint = (next_node, next_field.to_string());
            if !seen.insert(endpoint.clone()) {
                continue;
            }
            if let Some(stage) = view.stages.get(next_node) {
                out.push(trace_endpoint(
                    stage,
                    endpoint.1.clone(),
                    edge_kind,
                    hop + 1,
                ));
                queue.push_back((next_node, endpoint.1, hop + 1));
            }
        }
    }

    out.sort_by(|a, b| {
        a.hop
            .cmp(&b.hop)
            .then_with(|| a.stage_label.cmp(&b.stage_label))
            .then_with(|| a.field_name.cmp(&b.field_name))
    });
    out
}

fn trace_endpoint(
    stage: &StageView,
    field_name: String,
    edge_kind: FieldEdgeKind,
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
        hop,
    }
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
    }
}

fn edge_kind_attr(kind: FieldEdgeKind) -> &'static str {
    edge_kind_label(kind)
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
                is_aggregate_grain: true,
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
                },
                FieldEdge {
                    from_node: 0,
                    from_field: "x".to_string(),
                    to_node: 1,
                    to_field: "x2".to_string(),
                    kind: FieldEdgeKind::Derive,
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
        assert_eq!(emitted.upstream.len(), 1);
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
        let hop = &emitted.upstream[0];
        let hop_selection = hop.to_selected_field();
        assert_eq!(hop_selection.stage_id, hop.stage_id);
        assert_eq!(hop_selection.field_name, hop.field_name);
        let resolved = build_field_detail(&view, None, &hop_selection)
            .expect("trace hop resolves to a canvas field");
        assert_eq!(resolved.selection, hop_selection);
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
        assert!(detail.lineage_unavailable_reason.is_some());
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
            hop: 1,
        };
        assert!(build_field_detail(&view, None, &stale_hop.to_selected_field()).is_none());
    }
}
