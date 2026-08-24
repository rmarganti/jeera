//! Jira issue update domain module.

use crate::cli::UpdateArgs;
use crate::client::{
    JiraClient,
    types::{GetEditMetaRequest, UpdateIssueRequest},
};
use crate::error::AppError;
use crate::issue_fields::{
    CollectionChange, CommonIssueFieldInput, PrepareIssueFieldsError, prepare_common_fields,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Write;

#[derive(Debug)]
pub struct PreparedUpdateIssue {
    request: UpdateIssueRequest,
    dry_run: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct UpdateIssueOutput {
    issue_key: String,
    dry_run: bool,
    fields: BTreeMap<String, Value>,
}

pub fn execute(client: &JiraClient, args: &UpdateArgs) -> Result<UpdateIssueOutput, AppError> {
    let prepared = prepare(args)?;
    validate_against_edit_meta(client, &prepared)?;

    if !prepared.dry_run {
        client
            .update_issue(prepared.request())
            .map_err(|source| AppError::ExecuteUpdate { source })?;
    }

    Ok(output_from_prepared(prepared))
}

pub fn render_human(mut writer: impl Write, output: &UpdateIssueOutput) -> Result<(), AppError> {
    if output.dry_run {
        writeln!(writer, "Dry run: issue was not updated.")
            .map_err(|source| AppError::RenderOutput { source })?;
        writeln!(writer, "Would update {}.", output.issue_key)
            .map_err(|source| AppError::RenderOutput { source })?;
    } else {
        writeln!(writer, "Updated {}.", output.issue_key)
            .map_err(|source| AppError::RenderOutput { source })?;
    }
    Ok(())
}

impl PreparedUpdateIssue {
    pub fn request(&self) -> &UpdateIssueRequest {
        &self.request
    }
}

fn prepare(args: &UpdateArgs) -> Result<PreparedUpdateIssue, AppError> {
    if args.issue_key.trim().is_empty() {
        return Err(invalid("issue key cannot be empty"));
    }

    let components = collection_change(args.component.as_deref(), args.clear_components);
    let labels = collection_change(args.label.as_deref(), args.clear_labels);
    let fields = prepare_common_fields(CommonIssueFieldInput {
        summary: args.summary.as_deref(),
        body: args.body.as_deref(),
        body_file: args.body_file.as_deref(),
        clear_body: args.clear_body,
        components,
        labels,
        custom_fields: &args.field,
        reserved_fields: &["summary", "description", "components", "labels"],
    })
    .map_err(map_field_error)?;

    if fields.is_empty() {
        return Err(invalid("provide at least one field to update"));
    }

    Ok(PreparedUpdateIssue {
        request: UpdateIssueRequest {
            issue_id_or_key: args.issue_key.trim().to_string(),
            fields,
        },
        dry_run: args.dry_run,
    })
}

fn collection_change(values: Option<&[String]>, clear: bool) -> CollectionChange<'_> {
    if clear {
        CollectionChange::Clear
    } else if let Some(values) = values {
        CollectionChange::Set(values)
    } else {
        CollectionChange::Unchanged
    }
}

fn map_field_error(error: PrepareIssueFieldsError) -> AppError {
    match error {
        PrepareIssueFieldsError::Invalid(reason) => invalid(reason),
        PrepareIssueFieldsError::ReadInput { source } => AppError::ReadInput { source },
    }
}

fn validate_against_edit_meta(
    client: &JiraClient,
    prepared: &PreparedUpdateIssue,
) -> Result<(), AppError> {
    let meta = client
        .get_edit_meta(&GetEditMetaRequest {
            issue_id_or_key: prepared.request.issue_id_or_key.clone(),
        })
        .map_err(|source| AppError::ExecuteUpdate { source })?;

    validate_editable_fields(&prepared.request.fields, &meta.fields)
}

fn validate_editable_fields(
    requested: &BTreeMap<String, Value>,
    editable: &BTreeMap<String, crate::client::types::FieldMetadataResponse>,
) -> Result<(), AppError> {
    for key in requested.keys() {
        let Some(field) = editable.get(key) else {
            return Err(invalid(format!(
                "field {key:?} is not editable for this issue"
            )));
        };
        if !field.operations.iter().any(|operation| operation == "set") {
            return Err(invalid(format!(
                "field {key:?} does not support the set operation"
            )));
        }
    }
    Ok(())
}

fn invalid(reason: impl Into<String>) -> AppError {
    AppError::InvalidUpdate {
        reason: reason.into(),
    }
}

fn output_from_prepared(prepared: PreparedUpdateIssue) -> UpdateIssueOutput {
    UpdateIssueOutput {
        issue_key: prepared.request.issue_id_or_key,
        dry_run: prepared.dry_run,
        fields: prepared.request.fields,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepares_friendly_fields_and_clear_operations() {
        let prepared = prepare(&UpdateArgs {
            issue_key: "GCCDEV-1".to_string(),
            summary: Some("New summary".to_string()),
            body: Some("New body".to_string()),
            component: Some(vec!["Homes".to_string()]),
            clear_labels: true,
            field: vec!["customfield_1=value".to_string()],
            ..Default::default()
        })
        .unwrap();

        let fields = &prepared.request().fields;
        assert_eq!(fields["summary"], "New summary");
        assert_eq!(fields["components"][0]["name"], "Homes");
        assert_eq!(fields["labels"], serde_json::json!([]));
        assert_eq!(fields["customfield_1"], "value");
        assert_eq!(fields["description"]["type"], "doc");
    }

    #[test]
    fn rejects_fields_that_do_not_support_set() {
        let requested = BTreeMap::from([("labels".to_string(), serde_json::json!([]))]);
        let editable = BTreeMap::from([(
            "labels".to_string(),
            crate::client::types::FieldMetadataResponse {
                required: false,
                name: "Labels".to_string(),
                key: "labels".to_string(),
                operations: vec!["add".to_string(), "remove".to_string()],
                schema: None,
                allowed_values: Vec::new(),
            },
        )]);

        let error = validate_editable_fields(&requested, &editable).unwrap_err();
        assert_eq!(
            error.to_string(),
            "invalid update request: field \"labels\" does not support the set operation"
        );
    }

    #[test]
    fn rejects_an_update_without_changes() {
        let error = prepare(&UpdateArgs {
            issue_key: "GCCDEV-1".to_string(),
            ..Default::default()
        })
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "invalid update request: provide at least one field to update"
        );
    }
}
