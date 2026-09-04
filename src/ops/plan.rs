use super::{HandleReference, Op, OpStage};
use crate::inspect::deep_inspect;
use crate::{CliError, CliResult, EXIT_VALIDATION_FAILED, ResolvedProject, validate_project};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A plan that can be serialized, validated, and replayed independently of
/// the CLI argv surface.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct OpPlan {
    pub(crate) ops: Vec<Op>,
}

impl Serialize for OpPlan {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut object = Map::new();
        object.insert(
            "schema".to_string(),
            Value::String(super::OPS_SCHEMA.to_string()),
        );
        object.insert(
            "ops".to_string(),
            serde_json::to_value(&self.ops).map_err(serde::ser::Error::custom)?,
        );
        Value::Object(object).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for OpPlan {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let mut object = value
            .as_object()
            .cloned()
            .ok_or_else(|| serde::de::Error::custom("operation plan must be a JSON object"))?;
        if let Some(schema) = object.get("schema").and_then(Value::as_str)
            && schema != super::OPS_SCHEMA
        {
            return Err(serde::de::Error::custom(format!(
                "operation plan schema must be {}",
                super::OPS_SCHEMA
            )));
        }
        let ops = object
            .remove("ops")
            .ok_or_else(|| serde::de::Error::custom("operation plan requires an ops array"))?;
        let ops = serde_json::from_value(ops).map_err(serde::de::Error::custom)?;
        Ok(Self { ops })
    }
}

impl OpPlan {
    pub(crate) fn new(ops: Vec<Op>) -> Self {
        Self { ops }
    }

    pub(crate) fn validate(&self, project: &ProjectIndex) -> Result<ValidatedPlan, PlanError> {
        let mut available = project.handles.clone();
        let mut declarations = BTreeMap::<String, usize>::new();
        let mut stages = BTreeMap::<OpStage, Vec<usize>>::new();
        let mut previous_stage = None;

        for (index, operation) in self.ops.iter().enumerate() {
            let stage = operation.stage();
            if let Some(previous) = previous_stage
                && stage < previous
            {
                return Err(PlanError::new(
                    "ops.stage_order",
                    format!(
                        "operation `{}` is in {} stage after a {} stage operation",
                        operation.tag(),
                        stage.name(),
                        previous.name()
                    ),
                    format!("/ops/{index}"),
                ));
            }
            previous_stage = Some(stage);

            if let Some(duplicate_index) = self
                .ops
                .iter()
                .take(index)
                .position(|candidate| candidate == operation)
            {
                return Err(PlanError::new(
                    "ops.duplicate_operation",
                    format!(
                        "operation `{}` duplicates operation at index {duplicate_index}",
                        operation.tag()
                    ),
                    format!("/ops/{index}"),
                )
                .with_related_pointer(format!("/ops/{duplicate_index}")));
            }

            for reference in operation.references() {
                validate_reference(index, reference, &available)?;
            }

            if let Some(handle) = operation.declared_handle() {
                if handle.trim().is_empty() {
                    return Err(PlanError::new(
                        "ops.empty_handle",
                        "declared operation handle must not be empty",
                        format!("/ops/{index}/handle"),
                    ));
                }
                if project.contains(handle) {
                    return Err(PlanError::new(
                        "ops.handle_collision",
                        format!("declared handle already exists in the project: {handle}"),
                        format!("/ops/{index}/handle"),
                    ));
                }
                if let Some(previous_index) = declarations.get(handle) {
                    return Err(PlanError::new(
                        "ops.duplicate_handle",
                        format!("declared handle is already produced by operation {previous_index}: {handle}"),
                        format!("/ops/{index}/handle"),
                    )
                    .with_related_pointer(format!("/ops/{previous_index}/handle")));
                }
                declarations.insert(handle.to_string(), index);
                available.insert(handle.to_string());
            }

            stages.entry(stage).or_default().push(index);
        }

        let validated_ops = self
            .ops
            .iter()
            .enumerate()
            .map(|(index, operation)| ValidatedOp {
                index,
                stage: operation.stage(),
                operation: operation.clone(),
            })
            .collect();
        let stages = stages
            .into_iter()
            .map(|(stage, operations)| PlanStage {
                stage: stage.number(),
                name: stage.name(),
                operations,
            })
            .collect();
        Ok(ValidatedPlan {
            ops: validated_ops,
            stages,
        })
    }
}

fn validate_reference(
    index: usize,
    reference: HandleReference<'_>,
    available: &BTreeSet<String>,
) -> Result<(), PlanError> {
    if reference.handle.trim().is_empty() || !available.contains(reference.handle) {
        return Err(PlanError::new(
            "ops.dangling_handle",
            format!(
                "referenced handle does not exist in the project or an earlier operation: {}",
                if reference.handle.is_empty() {
                    "<empty>"
                } else {
                    reference.handle
                }
            ),
            format!("/ops/{index}/{}", reference.field),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ValidatedPlan {
    pub(crate) ops: Vec<ValidatedOp>,
    pub(crate) stages: Vec<PlanStage>,
}

impl ValidatedPlan {
    pub(crate) fn operations(&self) -> impl Iterator<Item = &Op> {
        self.ops.iter().map(|operation| &operation.operation)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ValidatedOp {
    pub(crate) index: usize,
    pub(crate) stage: OpStage,
    pub(crate) operation: Op,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PlanStage {
    pub(crate) stage: u8,
    pub(crate) name: &'static str,
    pub(crate) operations: Vec<usize>,
}

/// Handles visible in an existing project. New declarations are added to a
/// private copy during plan validation, so later operations may reference
/// earlier declarations without making a speculative filesystem change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ProjectIndex {
    handles: BTreeSet<String>,
}

/// Compatibility name used by the bridge plan. The operation core keeps the
/// index deliberately small: kernels receive the working [`ResolvedProject`]
/// from [`Transaction`](crate::ops::transaction::Transaction), while plans
/// only need the stable handles already visible in the project.
pub(crate) type ProjectContext = ProjectIndex;

impl ProjectIndex {
    pub(crate) fn new<I, S>(handles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            handles: handles.into_iter().map(Into::into).collect(),
        }
    }

    pub(crate) fn empty() -> Self {
        Self::default()
    }

    pub(crate) fn contains(&self, handle: &str) -> bool {
        self.handles.contains(handle)
    }

    pub(crate) fn handles(&self) -> impl Iterator<Item = &str> {
        self.handles.iter().map(String::as_str)
    }

    /// Build an index from the same deep inspection records exposed to agents.
    /// The report root is included explicitly because deep inspection keeps it
    /// as a report record rather than a child handle.
    pub(crate) fn from_project(project: &ResolvedProject) -> CliResult<Self> {
        let validation = validate_project(project)?;
        let deep = deep_inspect(project, &validation)?;
        let mut handles = BTreeSet::from(["report:main".to_string()]);
        if let Some(values) = deep["handles"].as_array() {
            handles.extend(
                values
                    .iter()
                    .filter_map(|value| value["handle"].as_str().map(ToOwned::to_owned)),
            );
        }
        Ok(Self { handles })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlanError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) pointer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) related_pointer: Option<String>,
}

impl PlanError {
    fn new(code: &'static str, message: impl Into<String>, pointer: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            pointer: pointer.into(),
            related_pointer: None,
        }
    }

    fn with_related_pointer(mut self, pointer: String) -> Self {
        self.related_pointer = Some(pointer);
        self
    }

    pub(crate) fn as_cli_error(&self) -> CliError {
        CliError::new(self.code, EXIT_VALIDATION_FAILED, self.message.clone())
            .with_pointer(self.pointer.clone())
    }
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}: {}",
            self.code, self.pointer, self.message
        )
    }
}

impl std::error::Error for PlanError {}

#[cfg(test)]
mod tests {
    use super::super::{AddMeasure, AddVisual, Op};
    use super::*;

    fn measure(handle: &str) -> Op {
        Op::AddMeasure(AddMeasure {
            handle: handle.into(),
            table: "Sales".into(),
            name: "Revenue".into(),
            expression: "SUM(Sales[Revenue])".into(),
            format_string: None,
            format_string_definition: None,
            description: None,
            display_folder: None,
        })
    }

    fn visual(handle: &str, page: &str) -> Op {
        Op::AddVisual(AddVisual {
            handle: handle.into(),
            page: page.into(),
            visual_type: "card".into(),
            name: None,
            title: None,
            mode: None,
            single_select: None,
            position: None,
            bindings: Vec::new(),
        })
    }

    #[test]
    fn plan_accepts_earlier_declarations_and_emits_numbered_stages() {
        let plan = OpPlan::new(vec![
            measure("measure:Sales:Revenue"),
            visual(
                "visual:ReportSectionOverview:VisualContainerRevenue",
                "page:ReportSectionOverview",
            ),
        ]);
        let project = ProjectIndex::new(["page:ReportSectionOverview"]);
        let validated = plan.validate(&project).expect("valid plan");
        assert_eq!(validated.stages[0].stage, 0);
        assert_eq!(validated.stages[0].name, "model");
        assert_eq!(validated.stages[1].stage, 2);
        assert_eq!(validated.stages[1].operations, vec![1]);
    }

    #[test]
    fn plan_rejects_dangling_handle_with_field_pointer() {
        let plan = OpPlan::new(vec![visual(
            "visual:ReportSectionOverview:VisualContainerRevenue",
            "page:missing",
        )]);
        let error = plan
            .validate(&ProjectIndex::empty())
            .expect_err("missing page must fail");
        assert_eq!(error.code, "ops.dangling_handle");
        assert_eq!(error.pointer, "/ops/0/page");
    }

    #[test]
    fn plan_rejects_duplicate_declarations_and_existing_collisions() {
        let duplicate = OpPlan::new(vec![
            measure("measure:Sales:Revenue"),
            measure("measure:Sales:Revenue"),
        ]);
        let duplicate_error = duplicate
            .validate(&ProjectIndex::empty())
            .expect_err("duplicate declaration must fail");
        assert_eq!(duplicate_error.code, "ops.duplicate_operation");
        assert_eq!(duplicate_error.pointer, "/ops/1");
        assert_eq!(
            serde_json::to_value(&duplicate_error).expect("plan error")["relatedPointer"],
            "/ops/0"
        );

        let collision = OpPlan::new(vec![measure("measure:Sales:Revenue")]);
        let collision_error = collision
            .validate(&ProjectIndex::new(["measure:Sales:Revenue"]))
            .expect_err("existing handle collision must fail");
        assert_eq!(collision_error.code, "ops.handle_collision");
        assert_eq!(collision_error.pointer, "/ops/0/handle");
    }

    #[test]
    fn plan_rejects_wrong_stage_order_with_operation_pointer() {
        let plan = OpPlan::new(vec![
            visual(
                "visual:ReportSectionOverview:VisualContainerRevenue",
                "page:ReportSectionOverview",
            ),
            measure("measure:Sales:Revenue"),
        ]);
        let error = plan
            .validate(&ProjectIndex::new(["page:ReportSectionOverview"]))
            .expect_err("model after visual must fail");
        assert_eq!(error.code, "ops.stage_order");
        assert_eq!(error.pointer, "/ops/1");
    }
}
