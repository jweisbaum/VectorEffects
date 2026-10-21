//! Native bridge and shared plugin installer. Install commands are UI-only.
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use std::sync::Arc;
use whirlwind_mcp::{
    bridge::{Bridge, NativeLauncher, Timing},
    install::{App, Homes, Host},
    policy::Policy,
};
pub fn app() -> anyhow::Result<App> {
    Ok(App { id:"vectoreffects".into(), server:"vectoreffects".into(), title:"VectorEffects".into(), version:env!("CARGO_PKG_VERSION").into(),
        description:"Use VectorEffects to inspect, edit, calculate, import and export the live desktop project.".into(),
        executable:std::env::current_exe()?, args:vec!["--mcp".into()], skill:super::skill::skill_md(),
        tools:super::tools::effects::TABLE.iter().map(|(name,effect)|(name.to_string(),*effect)).collect(), })
}
pub fn homes() -> anyhow::Result<Homes> {
    Ok(Homes::new(
        directories::BaseDirs::new()
            .ok_or_else(|| anyhow::anyhow!("No home directory"))?
            .home_dir()
            .to_owned(),
    ))
}
pub fn host(client: super::clients::McpClient) -> Host {
    match client {
        super::clients::McpClient::Codex => Host::Codex,
        super::clients::McpClient::ClaudeCode => Host::ClaudeCode,
        super::clients::McpClient::ClaudeDesktop => Host::ClaudeDesktop,
    }
}
pub fn connect(
    client: super::clients::McpClient,
    policy: Policy,
    legacy_url: &str,
) -> anyhow::Result<()> {
    whirlwind_mcp::install::connect_with_legacy(
        &app()?,
        &homes()?,
        host(client),
        policy,
        Some(legacy_url),
    )?;
    Ok(())
}
pub fn bridge() -> anyhow::Result<()> {
    let dir = crate::paths::AppPaths::resolve()?.config_dir.join("mcp");
    let info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
        .with_server_info(Implementation::new(
            "VectorEffects",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(super::tools::guide::INSTRUCTIONS);
    let launcher = NativeLauncher {
        executable: std::env::current_exe()?,
        identifier: "com.vectoreffects.desktop".into(),
        args: Vec::new(),
    };
    let bridge = Bridge::new(dir.clone(), Arc::new(launcher), Timing::DEFAULT)
        .map_err(|_| anyhow::anyhow!("Could not create local MCP bridge"))?
        .with_catalog(info, super::tools::VectorEffects::<tauri::Wry>::catalogue());
    let client = std::env::args().skip_while(|a| a != "--client").nth(1);
    let bridge = if let Some(client) =
        client.filter(|c| ["codex", "claude_code", "claude_desktop"].contains(&c.as_str()))
    {
        bridge.with_connection(dir.join(format!("client-{client}.json")))
    } else {
        bridge
    };
    whirlwind_mcp::bridge::run(bridge)
}

#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "McpConnection.ts")]
pub struct McpConnection {
    pub client: super::clients::McpClient,
    pub state: String,
    pub available: bool,
    pub detail: String,
}
#[tauri::command(async)]
pub fn mcp_connections(
    state: tauri::State<'_, crate::commands::AppState>,
) -> crate::error::Result<Vec<McpConnection>> {
    let app = app().map_err(|e| crate::error::AppError::Internal(e.to_string()))?;
    let homes = homes().map_err(|e| crate::error::AppError::Internal(e.to_string()))?;
    let dir = state.paths.config_dir.join("mcp");
    Ok(super::clients::McpClient::available()
        .into_iter()
        .map(|client| {
            let connection =
                whirlwind_mcp::install::verified_status(&app, &homes, host(client), &dir);
            McpConnection {
                client,
                state: connection.state,
                available: connection.available,
                detail: connection.detail,
            }
        })
        .collect())
}
#[tauri::command(async)]
pub fn mcp_connection_action(
    state: tauri::State<'_, crate::commands::AppState>,
    client: super::clients::McpClient,
    action: String,
) -> crate::error::Result<()> {
    let run = || -> anyhow::Result<()> {
        match action.as_str() {
            "disconnect" => {
                whirlwind_mcp::install::disconnect(&app()?, &homes()?, host(client))?;
            }
            "test" => whirlwind_mcp::install::test(&state.paths.config_dir.join("mcp"))?,
            _ => anyhow::bail!("Unknown connection action"),
        }
        Ok(())
    };
    run().map_err(|e| crate::error::AppError::Internal(e.to_string()))
}

pub fn journal(dir: &std::path::Path) -> Result<Arc<whirlwind_mcp::jobs::Journal>, String> {
    use std::sync::{Mutex, OnceLock};
    static JOURNALS: OnceLock<
        Mutex<std::collections::BTreeMap<std::path::PathBuf, Arc<whirlwind_mcp::jobs::Journal>>>,
    > = OnceLock::new();
    let mut journals = JOURNALS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "Job lock failed".to_string())?;
    if let Some(journal) = journals.get(dir) {
        return Ok(journal.clone());
    }
    let journal = Arc::new(
        whirlwind_mcp::jobs::Journal::open(dir.join("jobs.json")).map_err(|e| e.to_string())?,
    );
    journals.insert(dir.to_owned(), journal.clone());
    Ok(journal)
}
