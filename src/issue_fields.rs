//! Shared preparation for the friendly issue field interface.

use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read};
use thiserror::Error;

pub enum CollectionChange<'a> {
    Unchanged,
    Set(&'a [String]),
    Clear,
}

pub struct CommonIssueFieldInput<'a> {
    pub summary: Option<&'a str>,
    pub body: Option<&'a str>,
    pub body_file: Option<&'a str>,
    pub clear_body: bool,
    pub components: CollectionChange<'a>,
    pub labels: CollectionChange<'a>,
    pub custom_fields: &'a [String],
    pub reserved_fields: &'a [&'a str],
}

#[derive(Debug, Error)]
pub enum PrepareIssueFieldsError {
    #[error("{0}")]
    Invalid(String),
    #[error("while reading input: {source}")]
    ReadInput {
        #[source]
        source: io::Error,
    },
}

pub fn prepare_common_fields(
    input: CommonIssueFieldInput<'_>,
) -> Result<BTreeMap<String, Value>, PrepareIssueFieldsError> {
    validate_optional("summary", input.summary)?;
    validate_optional("body", input.body)?;
    validate_optional("body-file", input.body_file)?;

    let mut fields = BTreeMap::new();
    if let Some(summary) = input.summary {
        fields.insert("summary".to_string(), json!(summary.trim()));
    }

    if input.clear_body {
        fields.insert("description".to_string(), Value::Null);
    } else if let Some(body) = read_body(input.body, input.body_file)?
        && !body.trim().is_empty()
    {
        fields.insert("description".to_string(), text_to_adf(&body));
    }

    insert_collection(
        &mut fields,
        "components",
        input.components,
        |value| json!({ "name": value }),
    )?;
    insert_collection(&mut fields, "labels", input.labels, |value| json!(value))?;

    let reserved = input
        .reserved_fields
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    for assignment in input.custom_fields {
        let (key, value) = parse_field_assignment(assignment)?;
        if reserved.contains(key.as_str()) || fields.contains_key(&key) {
            return Err(PrepareIssueFieldsError::Invalid(format!(
                "--field cannot override built-in field {key:?}"
            )));
        }
        fields.insert(key, Value::String(value));
    }

    Ok(fields)
}

fn insert_collection(
    fields: &mut BTreeMap<String, Value>,
    key: &str,
    change: CollectionChange<'_>,
    encode: impl Fn(&str) -> Value,
) -> Result<(), PrepareIssueFieldsError> {
    match change {
        CollectionChange::Unchanged => {}
        CollectionChange::Clear => {
            fields.insert(key.to_string(), json!([]));
        }
        CollectionChange::Set(values) => {
            validate_repeated(key.trim_end_matches('s'), values)?;
            fields.insert(
                key.to_string(),
                Value::Array(values.iter().map(|value| encode(value.trim())).collect()),
            );
        }
    }
    Ok(())
}

fn read_body(
    body: Option<&str>,
    body_file: Option<&str>,
) -> Result<Option<String>, PrepareIssueFieldsError> {
    if let Some(body) = body {
        return Ok(Some(body.to_string()));
    }
    let Some(path) = body_file else {
        return Ok(None);
    };
    if path == "-" {
        let mut body = String::new();
        io::stdin()
            .read_to_string(&mut body)
            .map_err(|source| PrepareIssueFieldsError::ReadInput { source })?;
        return Ok(Some(body));
    }
    fs::read_to_string(path)
        .map(Some)
        .map_err(|source| PrepareIssueFieldsError::ReadInput { source })
}

fn parse_field_assignment(value: &str) -> Result<(String, String), PrepareIssueFieldsError> {
    let Some((key, value)) = value.split_once('=') else {
        return Err(PrepareIssueFieldsError::Invalid(
            "--field values must use KEY=VALUE".to_string(),
        ));
    };
    let key = key.trim();
    if key.is_empty() {
        return Err(PrepareIssueFieldsError::Invalid(
            "--field keys cannot be empty".to_string(),
        ));
    }
    if value.trim().is_empty() {
        return Err(PrepareIssueFieldsError::Invalid(format!(
            "--field {key} value cannot be empty"
        )));
    }
    Ok((key.to_string(), value.trim().to_string()))
}

fn text_to_adf(text: &str) -> Value {
    let content = text
        .split("\n\n")
        .filter_map(|paragraph| {
            let paragraph = paragraph.trim();
            if paragraph.is_empty() {
                return None;
            }
            let mut content = Vec::new();
            for (index, line) in paragraph.lines().enumerate() {
                if index > 0 {
                    content.push(json!({ "type": "hardBreak" }));
                }
                if !line.is_empty() {
                    content.push(json!({ "type": "text", "text": line }));
                }
            }
            Some(json!({ "type": "paragraph", "content": content }))
        })
        .collect();
    let mut document = Map::new();
    document.insert("type".to_string(), json!("doc"));
    document.insert("version".to_string(), json!(1));
    document.insert("content".to_string(), Value::Array(content));
    Value::Object(document)
}

fn validate_optional(name: &str, value: Option<&str>) -> Result<(), PrepareIssueFieldsError> {
    if value.is_some_and(|value| value.trim().is_empty()) {
        Err(PrepareIssueFieldsError::Invalid(format!(
            "--{name} cannot be empty"
        )))
    } else {
        Ok(())
    }
}

fn validate_repeated(name: &str, values: &[String]) -> Result<(), PrepareIssueFieldsError> {
    if values.iter().any(|value| value.trim().is_empty()) {
        Err(PrepareIssueFieldsError::Invalid(format!(
            "--{name} cannot contain empty values"
        )))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepares_the_shared_friendly_field_shape() {
        let fields = prepare_common_fields(CommonIssueFieldInput {
            summary: Some("Summary"),
            body: Some("One\nTwo"),
            body_file: None,
            clear_body: false,
            components: CollectionChange::Set(&["Homes".to_string()]),
            labels: CollectionChange::Clear,
            custom_fields: &["customfield_1=value".to_string()],
            reserved_fields: &["summary", "description", "components", "labels"],
        })
        .unwrap();

        assert_eq!(fields["summary"], "Summary");
        assert_eq!(fields["components"][0]["name"], "Homes");
        assert_eq!(fields["labels"], json!([]));
        assert_eq!(
            fields["description"]["content"][0]["content"][1]["type"],
            "hardBreak"
        );
    }
}
