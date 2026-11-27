use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::{Tool, ToolContext, ToolError, ToolOutput, ToolPreview, ToolRisk};

pub const ASK_USER_TOOL_NAME: &str = "ask_user";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionArgs {
    pub question: String,
    pub options: Vec<String>,
    pub allow_multiple: bool,
}

pub fn parse_question(args: &Value) -> Result<QuestionArgs, ToolError> {
    let question = args
        .get("question")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::InvalidArgs("missing non-empty 'question'".into()))?
        .to_string();

    let options: Vec<String> = args
        .get("options")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .take(9)
                .collect()
        })
        .unwrap_or_default();

    let allow_multiple = args
        .get("allow_multiple")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    Ok(QuestionArgs {
        question,
        options,
        allow_multiple,
    })
}

pub struct AskUserTool;

#[async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &'static str {
        ASK_USER_TOOL_NAME
    }

    fn description(&self) -> &'static str {
        "Ask the user a clarifying question when requirements are ambiguous or a real design decision is needed. Provide 2-5 concise options; the user can pick one (or several if allow_multiple is true) or type a custom answer."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "Clear, concise question for the user."
                },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of 2-5 concrete choices. The user can always type a custom answer instead."
                },
                "allow_multiple": {
                    "type": "boolean",
                    "description": "When true, the user may select more than one option."
                }
            },
            "required": ["question"]
        })
    }

    fn risk_level(&self) -> ToolRisk {
        ToolRisk::ReadOnly
    }

    fn preview(&self, _ctx: &ToolContext, args: &Value) -> Result<ToolPreview, ToolError> {
        let parsed = parse_question(args)?;
        Ok(ToolPreview {
            summary: format!("ask_user: {}", parsed.question),
            diff: None,
        })
    }

    async fn execute(&self, _ctx: &ToolContext, _args: &Value) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::err(
            "ask_user requires an interactive agent session.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_question_with_options_and_flags() {
        let q = parse_question(&json!({
            "question": "Which database?",
            "options": ["SQLite", "PostgreSQL", ""],
            "allow_multiple": true
        }))
        .unwrap();
        assert_eq!(q.question, "Which database?");
        assert_eq!(q.options, vec!["SQLite", "PostgreSQL"]);
        assert!(q.allow_multiple);
    }

    #[test]
    fn rejects_empty_question() {
        assert!(parse_question(&json!({ "question": "   " })).is_err());
    }
}
