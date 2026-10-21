//! Shared tool-failure shape. Text stays human-readable; clients can inspect structured fields.
use rmcp::model::{CallToolResult, ContentBlock};
pub fn refused(message: impl Into<String>) -> CallToolResult {
    let message = message.into();
    let mut result = CallToolResult::error(vec![ContentBlock::text(message.clone())]);
    result.structured_content = Some(
        serde_json::json!({"error":{"code":"operation_refused","message":message,"retryable":false},"contract_version":crate::CONTRACT_VERSION}),
    );
    result
}
