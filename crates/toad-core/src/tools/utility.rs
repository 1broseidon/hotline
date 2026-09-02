use super::ToolError;
use chrono::Local;
use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
pub struct CalculateArgs {
    operation: String,
    left: f64,
    right: f64,
}

pub struct Calculator;

impl Tool for Calculator {
    const NAME: &'static str = "calculator";
    type Error = ToolError;
    type Args = CalculateArgs;
    type Output = f64;

    fn description(&self) -> String {
        "Perform arithmetic. Supported operations are add, subtract, multiply, and divide."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["add", "subtract", "multiply", "divide"]
                },
                "left": { "type": "number" },
                "right": { "type": "number" }
            },
            "required": ["operation", "left", "right"],
            "additionalProperties": false
        })
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        error.into_execution_error()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        calculate(args)
    }
}

fn calculate(args: CalculateArgs) -> Result<f64, ToolError> {
    match args.operation.as_str() {
        "add" => Ok(args.left + args.right),
        "subtract" => Ok(args.left - args.right),
        "multiply" => Ok(args.left * args.right),
        "divide" if args.right == 0.0 => Err(ToolError::new("Cannot divide by zero.")),
        "divide" => Ok(args.left / args.right),
        operation => Err(ToolError::new(format!(
            "Unsupported operation: {operation}"
        ))),
    }
}

#[derive(Deserialize)]
pub struct CurrentTimeArgs {}

pub struct CurrentTime;

impl Tool for CurrentTime {
    const NAME: &'static str = "current_time";
    type Error = ToolError;
    type Args = CurrentTimeArgs;
    type Output = String;

    fn description(&self) -> String {
        "Return the current local date, time, and UTC offset.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        error.into_execution_error()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        _args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        Ok(Local::now().to_rfc3339())
    }
}

#[cfg(test)]
mod tests {
    use super::{CalculateArgs, calculate};

    #[test]
    fn calculator_rejects_division_by_zero() {
        let result = calculate(CalculateArgs {
            operation: "divide".to_string(),
            left: 10.0,
            right: 0.0,
        });

        assert_eq!(result.unwrap_err().to_string(), "Cannot divide by zero.");
    }

    #[test]
    fn calculator_multiplies() {
        let result = calculate(CalculateArgs {
            operation: "multiply".to_string(),
            left: 6.0,
            right: 7.0,
        });

        assert_eq!(result.unwrap(), 42.0);
    }
}
