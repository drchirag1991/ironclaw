//! JSON Schema validation for workspace documents.
//!
//! When a document (or its folder `.config`) carries a `schema` field in
//! metadata, all write operations validate the new content against that schema
//! before persisting. This enables typed, structured storage within the
//! workspace — settings, extension configs, and other system documents can
//! declare their expected shape and reject malformed writes at the boundary.

use crate::error::WorkspaceError;

/// Validate `content` as JSON against a JSON Schema.
///
/// Returns `Ok(())` if the content is valid JSON that conforms to the schema.
/// Returns `WorkspaceError::SchemaValidation` if:
/// - `content` is not valid JSON (schema implies JSON content)
/// - The parsed JSON does not conform to the schema
///
/// The `path` argument is used only for error messages.
pub fn validate_content_against_schema(
    path: &str,
    content: &str,
    schema: &serde_json::Value,
) -> Result<(), WorkspaceError> {
    let instance: serde_json::Value =
        serde_json::from_str(content).map_err(|e| WorkspaceError::SchemaValidation {
            path: path.to_string(),
            errors: vec![format!("content is not valid JSON: {e}")],
        })?;

    // NOTE: `jsonschema::validate` recompiles the schema on every call. This
    // is intentionally not cached today: schema-validated writes are limited
    // to settings/extension/skill state, which are rare user-initiated
    // operations (not a hot path). If schema validation moves into a frequent
    // write path, build a `Validator` once per distinct schema (e.g., via
    // `OnceCell`/`DashMap` keyed on the schema's canonical JSON) and call
    // `Validator::validate` here instead.
    jsonschema::validate(schema, &instance).map_err(|e| WorkspaceError::SchemaValidation {
        path: path.to_string(),
        errors: vec![e.to_string()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn valid_json_passes_schema() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" }
            },
            "required": ["name"]
        });
        let content = r#"{"name": "Alice", "age": 30}"#;
        assert!(validate_content_against_schema("test.json", content, &schema).is_ok());
    }

    #[test]
    fn invalid_json_fails() {
        let schema = json!({"type": "object"});
        let content = "not json at all";
        let err = validate_content_against_schema("test.json", content, &schema).unwrap_err();
        match err {
            WorkspaceError::SchemaValidation { path, errors } => {
                assert_eq!(path, "test.json");
                assert!(errors[0].contains("not valid JSON"));
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn missing_required_field_fails() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "required": ["name"]
        });
        let content = r#"{"age": 30}"#;
        let err = validate_content_against_schema("test.json", content, &schema).unwrap_err();
        match err {
            WorkspaceError::SchemaValidation { errors, .. } => {
                assert!(!errors.is_empty());
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn wrong_type_fails() {
        let schema = json!({
            "type": "object",
            "properties": {
                "count": { "type": "integer" }
            }
        });
        let content = r#"{"count": "not a number"}"#;
        let err = validate_content_against_schema("test.json", content, &schema).unwrap_err();
        match err {
            WorkspaceError::SchemaValidation { errors, .. } => {
                assert!(!errors.is_empty());
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn scalar_schema_validates_scalar_content() {
        let schema = json!({"type": "string"});
        let content = r#""hello""#;
        assert!(validate_content_against_schema("test.json", content, &schema).is_ok());

        let content = "42";
        let err = validate_content_against_schema("test.json", content, &schema).unwrap_err();
        match err {
            WorkspaceError::SchemaValidation { .. } => {}
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn enum_validation() {
        let schema = json!({
            "type": "string",
            "enum": ["anthropic", "openai", "ollama"]
        });
        assert!(validate_content_against_schema("test.json", r#""anthropic""#, &schema).is_ok());
        assert!(validate_content_against_schema("test.json", r#""unknown""#, &schema).is_err());
    }

    #[test]
    fn empty_object_passes_permissive_schema() {
        let schema = json!({"type": "object"});
        assert!(validate_content_against_schema("test.json", "{}", &schema).is_ok());
    }
}
