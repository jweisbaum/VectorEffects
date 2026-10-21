//! Isolated package/stdio fixture; never installs into a real host profile.
use std::{path::PathBuf, sync::Arc};
use whirlwind_mcp::{
    bridge::{Bridge, Launch, Timing},
    install::{App, Homes},
    policy::Effect,
};
struct NeverLaunch;
impl Launch for NeverLaunch {
    fn launch(&self) -> Result<(), String> {
        Err("No GUI in the contract fixture".into())
    }
}
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--mcp") {
        let tool = rmcp::model::Tool::new(
            "assistant_capabilities",
            "Read the integration contract",
            Arc::new(serde_json::from_value(
                serde_json::json!({"type":"object","properties":{},"additionalProperties":false}),
            )?),
        )
        .annotate(Effect::Read.annotations());
        let info = rmcp::model::ServerConfig::new(
            rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .build(),
        );
        let dir = tempfile::tempdir()?;
        let bridge = Bridge::new(
            dir.path().to_owned(),
            Arc::new(NeverLaunch),
            Timing::DEFAULT,
        )
        .map_err(|_| anyhow::anyhow!("Bridge creation failed"))?
        .with_catalog(info, vec![tool]);
        return whirlwind_mcp::bridge::run(bridge);
    }
    anyhow::ensure!(
        args.len() >= 3,
        "Usage: fixture <isolated-home> <plugin-id>"
    );
    let home = PathBuf::from(&args[1]);
    anyhow::ensure!(home.is_absolute(), "Use an absolute isolated directory");
    let app = App {
        id: args[2].clone(),
        server: args[2].replace('-', ""),
        title: args[2].clone(),
        version: "0.1.0".into(),
        description: "Isolated native MCP contract fixture".into(),
        executable: std::env::current_exe()?,
        args: vec!["--mcp".into()],
        skill: format!(
            "---\nname: {}\ndescription: Use the isolated MCP contract fixture.\n---\n\nRead assistant_capabilities.\n",
            args[2]
        ),
        tools: vec![("assistant_capabilities".into(), Effect::Read)],
    };
    let homes = Homes::isolated(home);
    if let Some(action) = args.get(3) {
        let host = match args.get(4).map(String::as_str) {
            Some("codex") => whirlwind_mcp::install::Host::Codex,
            Some("claude_code") => whirlwind_mcp::install::Host::ClaudeCode,
            _ => anyhow::bail!("Use codex or claude_code"),
        };
        let connection = match action.as_str() {
            "connect" => whirlwind_mcp::install::connect(
                &app,
                &homes,
                host,
                whirlwind_mcp::policy::Policy::Workflow,
            )?,
            "disconnect" => whirlwind_mcp::install::disconnect(&app, &homes, host)?,
            "status" => whirlwind_mcp::install::status(&app, &homes, host),
            _ => anyhow::bail!("Use connect, disconnect or status"),
        };
        println!("{}", serde_json::to_string(&connection)?);
        return Ok(());
    }
    let root = whirlwind_mcp::install::package(&app, &homes)?;
    whirlwind_mcp::install::write_bundle(&app, &root.join("fixture.mcpb"))?;
    println!("{}", root.display());
    Ok(())
}
