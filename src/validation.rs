//! JSON Schema validation for workflow inputs.
//!
//! Workflows can define an input schema in their YAML to validate
//! incoming data before execution begins.
//!
//! ## Example
//!
//! ```yaml
//! name: order-processor
//! input_schema:
//!   type: object
//!   required:
//!     - order_id
//!     - items
//!   properties:
//!     order_id:
//!       type: string
//!       pattern: "^ORD-[0-9]+$"
//!     items:
//!       type: array
//!       minItems: 1
//!       items:
//!         type: object
//!         required:
//!           - sku
//!           - quantity
//!         properties:
//!           sku:
//!             type: string
//!           quantity:
//!             type: integer
//!             minimum: 1
//! ```

use jsonschema::{validator_for, ValidationError, Validator};
use serde_json::Value;

use crate::error::{Error, Result};

/// Compiled JSON Schema validator.
pub struct SchemaValidator {
    validator: Validator,
    schema: Value,
}

impl SchemaValidator {
    /// Compile a JSON Schema for validation.
    pub fn new(schema: Value) -> Result<Self> {
        let validator = validator_for(&schema)
            .map_err(|e| Error::Validation(format!("Invalid JSON Schema: {}", e)))?;

        Ok(Self { validator, schema })
    }

    /// Validate input data against the schema.
    pub fn validate(&self, input: &Value) -> Result<()> {
        let result = self.validator.validate(input);

        if let Err(error) = result {
            let error_message = format_validation_error(&error);
            return Err(Error::Validation(format!(
                "Input validation failed: {}",
                error_message
            )));
        }

        Ok(())
    }

    /// Check if input is valid without detailed errors.
    pub fn is_valid(&self, input: &Value) -> bool {
        self.validator.is_valid(input)
    }

    /// Get the original schema.
    pub fn schema(&self) -> &Value {
        &self.schema
    }
}

/// Format a validation error for display.
fn format_validation_error(error: &ValidationError) -> String {
    let path = error.instance_path.to_string();
    if path.is_empty() || path == "/" {
        error.to_string()
    } else {
        format!("at '{}': {}", path, error)
    }
}

/// Validate input against an optional schema.
pub fn validate_input(schema: Option<&Value>, input: &Value) -> Result<()> {
    match schema {
        Some(schema) => {
            let validator = SchemaValidator::new(schema.clone())?;
            validator.validate(input)
        }
        None => Ok(()), // No schema = no validation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_valid_object() {
        let schema = json!({
            "type": "object",
            "required": ["name", "age"],
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer", "minimum": 0 }
            }
        });

        let validator = SchemaValidator::new(schema).unwrap();

        let valid_input = json!({ "name": "Alice", "age": 30 });
        assert!(validator.validate(&valid_input).is_ok());
    }

    #[test]
    fn test_missing_required_field() {
        let schema = json!({
            "type": "object",
            "required": ["name", "age"],
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" }
            }
        });

        let validator = SchemaValidator::new(schema).unwrap();

        let invalid_input = json!({ "name": "Alice" });
        let result = validator.validate(&invalid_input);
        assert!(result.is_err());

        let error = result.unwrap_err().to_string();
        assert!(error.contains("age"));
    }

    #[test]
    fn test_wrong_type() {
        let schema = json!({
            "type": "object",
            "properties": {
                "count": { "type": "integer" }
            }
        });

        let validator = SchemaValidator::new(schema).unwrap();

        let invalid_input = json!({ "count": "not a number" });
        let result = validator.validate(&invalid_input);
        assert!(result.is_err());
    }

    #[test]
    fn test_pattern_validation() {
        let schema = json!({
            "type": "object",
            "properties": {
                "email": {
                    "type": "string",
                    "pattern": "^[a-z]+@[a-z]+\\.[a-z]+$"
                }
            }
        });

        let validator = SchemaValidator::new(schema).unwrap();

        let valid_input = json!({ "email": "test@example.com" });
        assert!(validator.validate(&valid_input).is_ok());

        let invalid_input = json!({ "email": "not-an-email" });
        assert!(validator.validate(&invalid_input).is_err());
    }

    #[test]
    fn test_array_validation() {
        let schema = json!({
            "type": "array",
            "minItems": 1,
            "items": { "type": "string" }
        });

        let validator = SchemaValidator::new(schema).unwrap();

        let valid_input = json!(["a", "b", "c"]);
        assert!(validator.validate(&valid_input).is_ok());

        let empty_array = json!([]);
        assert!(validator.validate(&empty_array).is_err());

        let wrong_type = json!([1, 2, 3]);
        assert!(validator.validate(&wrong_type).is_err());
    }

    #[test]
    fn test_number_constraints() {
        let schema = json!({
            "type": "object",
            "properties": {
                "quantity": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100
                }
            }
        });

        let validator = SchemaValidator::new(schema).unwrap();

        let valid = json!({ "quantity": 50 });
        assert!(validator.validate(&valid).is_ok());

        let too_small = json!({ "quantity": 0 });
        assert!(validator.validate(&too_small).is_err());

        let too_large = json!({ "quantity": 101 });
        assert!(validator.validate(&too_large).is_err());
    }

    #[test]
    fn test_nested_object() {
        let schema = json!({
            "type": "object",
            "required": ["user"],
            "properties": {
                "user": {
                    "type": "object",
                    "required": ["id"],
                    "properties": {
                        "id": { "type": "integer" },
                        "name": { "type": "string" }
                    }
                }
            }
        });

        let validator = SchemaValidator::new(schema).unwrap();

        let valid = json!({ "user": { "id": 123, "name": "Alice" } });
        assert!(validator.validate(&valid).is_ok());

        let missing_id = json!({ "user": { "name": "Alice" } });
        assert!(validator.validate(&missing_id).is_err());
    }

    #[test]
    fn test_validate_input_no_schema() {
        let input = json!({ "anything": "goes" });
        assert!(validate_input(None, &input).is_ok());
    }

    #[test]
    fn test_validate_input_with_schema() {
        let schema = json!({
            "type": "object",
            "required": ["key"]
        });

        let valid_input = json!({ "key": "value" });
        assert!(validate_input(Some(&schema), &valid_input).is_ok());

        let invalid_input = json!({});
        assert!(validate_input(Some(&schema), &invalid_input).is_err());
    }

    #[test]
    fn test_is_valid() {
        let schema = json!({
            "type": "string"
        });

        let validator = SchemaValidator::new(schema).unwrap();

        assert!(validator.is_valid(&json!("hello")));
        assert!(!validator.is_valid(&json!(123)));
    }

    #[test]
    fn test_invalid_schema() {
        let invalid_schema = json!({
            "type": "not-a-valid-type"
        });

        // jsonschema crate may or may not reject this at compile time
        // depending on the version, so we just check it doesn't panic
        let _ = SchemaValidator::new(invalid_schema);
    }
}
