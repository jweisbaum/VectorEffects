//! Effects describe reality; host policy, never misleading annotations, controls prompts.
use rmcp::model::{MetaObject, Tool, ToolAnnotations};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Read,
    ReadNetwork,
    Edit,
    View,
    Files,
    Network,
    Discard,
    Paid,
    Anything,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    #[default]
    Workflow,
    AlwaysAsk,
    Unrestricted,
}
impl Effect {
    pub fn automatic(self, policy: Policy) -> bool {
        match policy {
            Policy::AlwaysAsk => false,
            Policy::Unrestricted => true,
            Policy::Workflow => matches!(
                self,
                Self::Read
                    | Self::ReadNetwork
                    | Self::Edit
                    | Self::View
                    | Self::Files
                    | Self::Network
            ),
        }
    }
    pub fn annotations(self) -> ToolAnnotations {
        let (read, destructive, idempotent, network) = match self {
            Self::Read => (true, false, true, false),
            Self::ReadNetwork => (true, false, true, true),
            Self::Edit => (false, false, false, false),
            Self::View => (false, false, true, false),
            Self::Files | Self::Discard => (false, true, false, false),
            Self::Network => (false, false, false, true),
            Self::Paid | Self::Anything => (false, true, false, true),
        };
        ToolAnnotations::from_raw(
            None,
            Some(read),
            Some(destructive),
            Some(idempotent),
            Some(network),
        )
    }
}
pub fn classify(tool: &mut Tool, effect: Effect) {
    tool.annotations = Some(effect.annotations());
    let meta = tool.meta.get_or_insert_with(MetaObject::new);
    meta.0.insert(
        "com.whirlwind/effect".into(),
        serde_json::to_value(effect).unwrap_or(Value::Null),
    );
    meta.0
        .insert("com.whirlwind/contractVersion".into(), 1.into());
}
/// Strip authorization/configuration material from every assistant-facing structured result.
/// This also catches settings returned by mutation commands. UI serialization is unchanged.
pub fn redact(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|key, _| {
                !matches!(
                    key.to_ascii_lowercase().as_str(),
                    "token"
                        | "password"
                        | "client_secret"
                        | "aws_secret_access_key"
                        | "database_url"
                        | "connection_string"
                        | "access_token"
                        | "refresh_token"
                        | "authorization"
                        | "api_key"
                        | "secret_access_key"
                        | "session_token"
                        | "claude_rules"
                        | "mcp"
                )
            });
            for value in map.values_mut() {
                redact(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                redact(value);
            }
        }
        _ => {}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workflow_allows_routine_work_without_falsifying_effects() {
        for effect in [
            Effect::Read,
            Effect::Edit,
            Effect::View,
            Effect::Files,
            Effect::Network,
        ] {
            assert!(effect.automatic(Policy::Workflow));
        }
        for effect in [Effect::Paid, Effect::Discard, Effect::Anything] {
            assert!(!effect.automatic(Policy::Workflow));
        }
        assert_eq!(Effect::Network.annotations().open_world_hint, Some(true));
        assert_eq!(Effect::Files.annotations().destructive_hint, Some(true));
    }
    #[test]
    fn settings_are_redacted_at_any_depth() {
        let mut value = serde_json::json!({"result":{"theme":"dark","mcp":{"token":"secret"}},"items":[{"token":"secret","name":"boat"}]});
        redact(&mut value);
        assert!(!value.to_string().contains("secret"));
        assert_eq!(value["result"]["theme"], "dark");
    }
}
