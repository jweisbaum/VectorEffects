//! The only module that changes host configuration. Call exclusively from app Settings.
use crate::policy::{Effect, Policy};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Clone)]
pub struct App {
    pub id: String,
    pub server: String,
    pub title: String,
    pub version: String,
    pub description: String,
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub skill: String,
    pub tools: Vec<(String, Effect)>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Host {
    Codex,
    ClaudeCode,
    ClaudeDesktop,
}
impl Host {
    pub fn key(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude_code",
            Self::ClaudeDesktop => "claude_desktop",
        }
    }
    fn binary(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude",
            Self::ClaudeDesktop => "Claude",
        }
    }
}
#[derive(Debug, Clone)]
pub struct Homes {
    pub home: PathBuf,
    pub codex: PathBuf,
    pub claude: PathBuf,
    pub marketplace: PathBuf,
    isolated_profile: bool,
}
impl Homes {
    pub fn new(home: PathBuf) -> Self {
        Self {
            codex: std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex")),
            claude: std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".claude")),
            marketplace: home.join(".whirlwind/marketplace"),
            isolated_profile: false,
            home,
        }
    }
    pub fn isolated(home: PathBuf) -> Self {
        Self {
            codex: home.join(".codex"),
            claude: home.join(".claude"),
            marketplace: home.join(".whirlwind/marketplace"),
            isolated_profile: true,
            home,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub host: Host,
    pub state: String,
    pub available: bool,
    pub detail: String,
    pub plugin: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Receipt {
    version: String,
    executable: PathBuf,
    policy: Policy,
    #[serde(default)]
    rules: Vec<String>,
    #[serde(default)]
    approvals: BTreeMap<String, String>,
}
fn receipt_path(h: &Homes, app: &App, host: Host) -> PathBuf {
    h.marketplace
        .join("receipts")
        .join(format!("{}-{}.json", app.id, host.key()))
}
fn receipt(h: &Homes, app: &App, host: Host) -> Result<Receipt> {
    match fs::read(receipt_path(h, app, host)) {
        Ok(b) => Ok(serde_json::from_slice(&b)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Receipt::default()),
        Err(e) => Err(e.into()),
    }
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let path = if path.is_symlink() {
        fs::canonicalize(path)?
    } else {
        path.to_owned()
    };
    let parent = path.parent().context("File has no parent")?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    if let Ok(meta) = fs::metadata(&path) {
        file.as_file().set_permissions(meta.permissions())?;
    }
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(&path).map_err(|e| e.error)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
/// Serialize our writers and detect an external edit before replacement. Keep an exact backup.
fn edit(path: &Path, change: impl FnOnce(&str) -> Result<String>) -> Result<()> {
    let original = match fs::read(path) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let text = std::str::from_utf8(original.as_deref().unwrap_or_default())?;
    let next = change(text)?;
    if next.as_bytes() == text.as_bytes() {
        return Ok(());
    }
    ensure!(
        fs::read(path).ok() == original,
        "Configuration changed while connecting. Retry; it was left untouched."
    );
    if let Some(original) = original {
        let backup = path.with_extension(format!("whirlwind-{}.bak", uuid::Uuid::new_v4()));
        atomic_write(&backup, &original)?;
    }
    atomic_write(path, next.as_bytes())
}
fn lock(h: &Homes) -> Result<fs::File> {
    fs::create_dir_all(&h.marketplace)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(h.marketplace.join("install.lock"))?;
    fs2::FileExt::try_lock_exclusive(&file)
        .context("Another Whirlwind app is connecting. Retry when it finishes.")?;
    Ok(file)
}
pub fn locate(host: Host, h: &Homes) -> Option<PathBuf> {
    if host == Host::ClaudeDesktop {
        return None;
    }
    let name = host.binary();
    let names = if cfg!(windows) {
        vec![format!("{name}.exe"), format!("{name}.cmd")]
    } else {
        vec![name.into()]
    };
    let mut dirs = vec![
        h.home.join(".local/bin"),
        h.home.join(".claude/local"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/Applications/Codex.app/Contents/Resources"),
        h.home.join("AppData/Roaming/npm"),
    ];
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.into_iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|p| p.is_file())
}
/// Run host CLIs without a shell, with a timeout and bounded output.
fn run(program: &Path, args: &[String], h: &Homes) -> Result<String> {
    fs::create_dir_all(&h.codex)?;
    fs::create_dir_all(&h.claude)?;
    super::process::run(
        program,
        args,
        &h.codex,
        (h.isolated_profile || std::env::var_os("CLAUDE_CONFIG_DIR").is_some())
            .then_some(h.claude.as_path()),
        Duration::from_secs(60),
    )
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| s.to_string()).collect()
}
/// One portable plugin directory usable by Codex and Claude Code, both launching native Rust.
pub fn package(app: &App, h: &Homes) -> Result<PathBuf> {
    ensure!(
        app.id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
        "Invalid plugin id"
    );
    let root = h.marketplace.join("plugins").join(&app.id);
    fs::create_dir_all(&root)?;
    let portable = json!({"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":app.id,"version":app.version,"description":app.description,"author":{"name":"Whirlwind"},"license":"PolyForm-Noncommercial-1.0.0"});
    let claude = json!({"name":app.id,"version":app.version,"description":app.description,"author":{"name":"Whirlwind"}});
    let launcher = if cfg!(windows) {
        "scripts/whirlwind-launch.exe"
    } else {
        "scripts/whirlwind-launch"
    };
    atomic_write(&root.join(launcher), crate::LAUNCHER)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root.join(launcher), fs::Permissions::from_mode(0o755))?;
    }
    let config = |host: Host| {
        let mut args = vec![app.executable.display().to_string()];
        args.extend(app.args.clone());
        args.extend(["--client".into(), host.key().into()]);
        let variable = if host == Host::Codex {
            "${PLUGIN_ROOT}"
        } else {
            "${CLAUDE_PLUGIN_ROOT}"
        };
        json!({"mcpServers":{&app.server:{"command":format!("{variable}/{launcher}"),"args":args}}})
    };
    let mcp = config(Host::Codex);
    let claude_mcp = config(Host::ClaudeCode);
    for (path, value) in [
        ("plugin.json", portable),
        (".claude-plugin/plugin.json", claude),
        ("mcp.json", mcp.clone()),
        (".mcp.json", claude_mcp),
    ] {
        atomic_write(&root.join(path), &serde_json::to_vec_pretty(&value)?)?;
    }
    atomic_write(
        &root.join(format!("skills/{}/SKILL.md", app.id)),
        app.skill.as_bytes(),
    )?;
    let mut codex = Vec::new();
    let mut claude = Vec::new();
    for entry in fs::read_dir(h.marketplace.join("plugins"))? {
        let entry = entry?;
        if let Ok(bytes) = fs::read(entry.path().join("plugin.json")) {
            let manifest: Value = serde_json::from_slice(&bytes)?;
            let id = manifest["name"].as_str().context("Missing plugin name")?;
            codex.push(json!({"name":id,"source":{"source":"local","path":format!("./plugins/{id}")},"policy":{"installation":"AVAILABLE","authentication":"ON_INSTALL"},"category":"Productivity"}));
            claude.push(json!({"name":id,"source":format!("./plugins/{id}"),"description":manifest["description"]}));
        }
    }
    codex.sort_by_key(|v| v["name"].as_str().unwrap_or_default().to_owned());
    claude.sort_by_key(|v| v["name"].as_str().unwrap_or_default().to_owned());
    atomic_write(
        &h.marketplace.join(".agents/plugins/marketplace.json"),
        &serde_json::to_vec_pretty(
            &json!({"name":"whirlwind","interface":{"displayName":"Whirlwind apps"},"plugins":codex}),
        )?,
    )?;
    atomic_write(
        &h.marketplace.join(".claude-plugin/marketplace.json"),
        &serde_json::to_vec_pretty(
            &json!({"name":"whirlwind","owner":{"name":"Whirlwind"},"plugins":claude}),
        )?,
    )?;
    Ok(root)
}
fn codex_policy(
    app: &App,
    h: &Homes,
    policy: Policy,
    previous: &Receipt,
    legacy_url: Option<&str>,
) -> Result<BTreeMap<String, String>> {
    let mut written = BTreeMap::new();
    edit(&h.codex.join("config.toml"), |text| {
        let mut doc = text.parse::<toml_edit::DocumentMut>()?;
        let plugin = format!("{}@whirlwind", app.id);
        // An existing direct entry must be known to belong to this app before migrating.
        if let Some(entry) = doc.get("mcp_servers").and_then(|x| x.get(&app.server)) {
            let command = entry.get("command").and_then(|v| v.as_str());
            let owned = command == app.executable.to_str()
                || entry
                    .get("url")
                    .and_then(|v| v.as_str())
                    .is_some_and(|url| legacy_url == Some(url));
            ensure!(
                owned,
                "An existing MCP server named {} belongs to another configuration. Rename it before connecting.",
                app.server
            );
            let legacy = entry.clone();
            for (key, value) in legacy
                .as_table_like()
                .context("MCP server is not a table")?
                .iter()
            {
                if !matches!(
                    key,
                    "command" | "args" | "url" | "http_headers" | "bearer_token_env_var" | "env"
                ) && doc["plugins"][&plugin]["mcp_servers"][&app.server]
                    .get(key)
                    .is_none()
                {
                    doc["plugins"][&plugin]["mcp_servers"][&app.server][key] = value.clone();
                }
            }
            doc["mcp_servers"]
                .as_table_like_mut()
                .context("Invalid MCP table")?
                .remove(&app.server);
        }
        let server = &mut doc["plugins"][&plugin]["mcp_servers"][&app.server];
        server["default_tools_approval_mode"] = toml_edit::value("prompt");
        for (name, effect) in &app.tools {
            let want = if effect.automatic(policy) {
                "approve"
            } else {
                "prompt"
            };
            let current = server
                .get("tools")
                .and_then(|v| v.get(name))
                .and_then(|v| v.get("approval_mode"))
                .and_then(|v| v.as_str());
            if current.is_none()
                || previous
                    .approvals
                    .get(name)
                    .is_some_and(|old| Some(old.as_str()) == current)
            {
                server["tools"][name]["approval_mode"] = toml_edit::value(want);
                written.insert(name.clone(), want.into());
            }
        }
        Ok(doc.to_string())
    })?;
    Ok(written)
}
fn claude_policy(app: &App, h: &Homes, policy: Policy, previous: &Receipt) -> Result<Vec<String>> {
    let mut written = Vec::new();
    edit(&h.claude.join("settings.json"), |text| {
        let mut doc: Value = if text.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(text)?
        };
        ensure!(doc.is_object(), "Claude settings must be an object");
        if doc.get("permissions").is_none() {
            doc["permissions"] = json!({});
        }
        ensure!(
            doc["permissions"].is_object(),
            "Claude permissions must be an object"
        );
        if doc["permissions"].get("allow").is_none() {
            doc["permissions"]["allow"] = json!([]);
        }
        let allow = doc["permissions"]["allow"]
            .as_array_mut()
            .context("Claude allow must be an array")?;
        allow.retain(|v| {
            !v.as_str()
                .is_some_and(|s| previous.rules.iter().any(|r| r == s))
        });
        for (name, effect) in &app.tools {
            if effect.automatic(policy) {
                let rule = format!("mcp__plugin_{}_{}__{}", app.id, app.server, name);
                if !allow.iter().any(|v| v.as_str() == Some(&rule)) {
                    allow.push(rule.clone().into());
                    written.push(rule);
                }
            }
        }
        Ok(serde_json::to_string_pretty(&doc)? + "\n")
    })?;
    Ok(written)
}
pub fn connect(app: &App, h: &Homes, host: Host, policy: Policy) -> Result<Connection> {
    connect_with_legacy(app, h, host, policy, None)
}
pub fn connect_with_legacy(
    app: &App,
    h: &Homes,
    host: Host,
    policy: Policy,
    legacy_url: Option<&str>,
) -> Result<Connection> {
    let _lock = lock(h)?;
    ensure!(
        app.executable.is_file(),
        "The installed application executable is missing"
    );
    // Keep the pre-install host configuration, including before the host CLI edits it.
    let config = if host == Host::Codex {
        h.codex.join("config.toml")
    } else {
        h.claude.join("settings.json")
    };
    if let Ok(bytes) = fs::read(&config) {
        atomic_write(
            &config.with_extension(format!("whirlwind-{}.bak", uuid::Uuid::new_v4())),
            &bytes,
        )?;
    }
    let root = package(app, h)?;
    let old = receipt(h, app, host)?;
    let plugin = format!("{}@whirlwind", app.id);
    if host == Host::ClaudeDesktop {
        let bundle = h.marketplace.join(format!("{}.mcpb", app.id));
        write_bundle(app, &bundle)?;
        #[cfg(target_os = "macos")]
        run(
            Path::new("/usr/bin/open"),
            &["-a".into(), "Claude".into(), bundle.display().to_string()],
            h,
        )?;
        #[cfg(windows)]
        run(
            Path::new("explorer.exe"),
            &[bundle.display().to_string()],
            h,
        )?;
        #[cfg(target_os = "linux")]
        anyhow::bail!("Claude Desktop is unavailable on Linux");
    } else {
        let cli = locate(host, h).with_context(|| format!("{} is not installed", host.binary()))?;
        let listing = run(&cli, &args(&["plugin", "marketplace", "list", "--json"]), h)?;
        // Refuse to replace a same-name marketplace from another publisher/location.
        let value: Value =
            serde_json::from_str(&listing).context("Could not inspect host marketplaces")?;
        let legacy_plugin = old_sync_marketplace(app, host, &value)?;
        let mut matches = Vec::new();
        named_objects(&value, "whirlwind", &mut matches);
        if !matches.is_empty() {
            ensure!(
                matches.len() == 1,
                "Ambiguous Whirlwind marketplace registration; nothing was replaced"
            );
            let entry = matches[0];
            let path = if host == Host::ClaudeCode {
                (entry["source"] == "directory")
                    .then(|| entry["path"].as_str())
                    .flatten()
            } else {
                ["root", "path", "source"]
                    .iter()
                    .find_map(|key| entry[*key].as_str())
            };
            ensure!(
                path.map(Path::new)
                    .is_some_and(|path| same_path(path, &h.marketplace)),
                "A different marketplace is named whirlwind; it was left unchanged"
            );
        } else {
            let mut add = args(&["plugin", "marketplace", "add"]);
            add.push(h.marketplace.display().to_string());
            run(&cli, &add, h)?;
        }
        // The legacy HTTP/stdio registration is removed through Claude's own CLI,
        // and only when its URL/command identifies this app exactly.
        let legacy_claude = if host == Host::ClaudeCode {
            legacy_claude_entry(app, h, legacy_url)?
        } else {
            false
        };
        let command = if host == Host::Codex {
            "add"
        } else {
            "install"
        };
        let mut install = args(&["plugin", command, &plugin]);
        if host == Host::ClaudeCode {
            install.extend(args(&["--scope", "user"]));
        }
        run(&cli, &install, h)?;
        if legacy_plugin {
            if host == Host::Codex {
                edit(&h.codex.join("config.toml"), |text| {
                    let mut doc = text.parse::<toml_edit::DocumentMut>()?;
                    if let Some(legacy) = doc
                        .get("plugins")
                        .and_then(|p| p.get("hindsight-sync@hindsight-sync"))
                        .and_then(|p| p.get("mcp_servers"))
                        .cloned()
                        && doc["plugins"][&plugin].get("mcp_servers").is_none()
                    {
                        doc["plugins"][&plugin]["mcp_servers"] = legacy;
                    }
                    Ok(doc.to_string())
                })?;
            }
            let installed = run(&cli, &args(&["plugin", "list", "--json"]), h)?;
            let listed: Value = serde_json::from_str(&installed)?;
            if contains_plugin(&listed, "hindsight-sync@hindsight-sync") {
                let mut remove = args(&[
                    "plugin",
                    if host == Host::Codex {
                        "remove"
                    } else {
                        "uninstall"
                    },
                    "hindsight-sync@hindsight-sync",
                ]);
                if host == Host::ClaudeCode {
                    remove.extend(args(&["--scope", "user"]));
                }
                run(&cli, &remove, h)?;
            }
        }
        if legacy_claude {
            run(
                &cli,
                &args(&["mcp", "remove", "--scope", "user", &app.server]),
                h,
            )?;
        }
    }
    let mut next = Receipt {
        version: app.version.clone(),
        executable: app.executable.clone(),
        policy,
        ..old.clone()
    };
    match host {
        Host::Codex => next.approvals = codex_policy(app, h, policy, &old, legacy_url)?,
        Host::ClaudeCode => next.rules = claude_policy(app, h, policy, &old)?,
        Host::ClaudeDesktop => {}
    }
    atomic_write(
        &receipt_path(h, app, host),
        &serde_json::to_vec_pretty(&next)?,
    )?;
    Ok(Connection {
        host,
        state: "restart_required".into(),
        available: true,
        detail: format!(
            "Plugin and workflow permissions installed. Restart the assistant to load {}.",
            root.display()
        ),
        plugin,
    })
}
pub fn status(app: &App, h: &Homes, host: Host) -> Connection {
    let available = if host == Host::ClaudeDesktop {
        #[cfg(target_os = "macos")]
        {
            Path::new("/Applications/Claude.app").is_dir()
                || h.home.join("Applications/Claude.app").is_dir()
        }
        #[cfg(windows)]
        {
            true
        } // The Windows extension handler is resolved by the OS when connecting.
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            false
        }
    } else {
        locate(host, h).is_some()
    };
    let saved = receipt(h, app, host).ok().filter(|r| !r.version.is_empty());
    let state = if !available {
        "unavailable"
    } else if let Some(r) = &saved {
        if r.executable == app.executable
            && r.version == app.version
            && host_registration(app, h, host)
            && h.marketplace
                .join("plugins")
                .join(&app.id)
                .join("mcp.json")
                .is_file()
        {
            "restart_required"
        } else {
            "needs_repair"
        }
    } else if receipt_path(h, app, host).exists()
        || (host != Host::ClaudeDesktop && host_registration(app, h, host))
    {
        "needs_repair"
    } else {
        "disconnected"
    };
    Connection {
        host,
        state: state.into(),
        available,
        detail: match state {
            "unavailable" => "Install this assistant first.",
            "needs_repair" => "The app or plugin moved. Repair reconnects it.",
            "restart_required" => {
                "Plugin installed. Test from the assistant to verify the live connection."
            }
            _ => "Ready to connect.",
        }
        .into(),
        plugin: format!("{}@whirlwind", app.id),
    }
}
pub fn disconnect(app: &App, h: &Homes, host: Host) -> Result<Connection> {
    let _lock = lock(h)?;
    let old = receipt(h, app, host)?;
    if host == Host::ClaudeDesktop {
        anyhow::bail!("Remove this extension in Claude Desktop > Settings > Extensions.");
    }
    let cli = locate(host, h).context("Assistant command was not found")?;
    let plugin = format!("{}@whirlwind", app.id);
    let mut remove = args(&[
        "plugin",
        if host == Host::Codex {
            "remove"
        } else {
            "uninstall"
        },
        &plugin,
    ]);
    if host == Host::ClaudeCode {
        remove.extend(args(&["--scope", "user"]));
    }
    run(&cli, &remove, h)?;
    if host == Host::ClaudeCode {
        claude_policy(app, h, Policy::AlwaysAsk, &old)?;
    } else {
        edit(&h.codex.join("config.toml"), |text| {
            let mut doc = text.parse::<toml_edit::DocumentMut>()?;
            if let Some(tools) = doc
                .get_mut("plugins")
                .and_then(|p| p.get_mut(&plugin))
                .and_then(|p| p.get_mut("mcp_servers"))
                .and_then(|s| s.get_mut(&app.server))
                .and_then(|s| s.get_mut("tools"))
                .and_then(|t| t.as_table_like_mut())
            {
                for (name, owned) in &old.approvals {
                    if tools
                        .get(name)
                        .and_then(|t| t.get("approval_mode"))
                        .and_then(|v| v.as_str())
                        == Some(owned.as_str())
                    {
                        tools.remove(name);
                    }
                }
            }
            Ok(doc.to_string())
        })?;
    }
    let path = receipt_path(h, app, host);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(status(app, h, host))
}
/// Bundle launches the installed native executable. No copied token, JavaScript or shell runtime.
pub fn write_bundle(app: &App, path: &Path) -> Result<()> {
    let mut launch_args = vec![app.executable.display().to_string()];
    launch_args.extend(app.args.clone());
    launch_args.extend(["--client".into(), "claude_desktop".into()]);
    let entry = if cfg!(windows) {
        "server/whirlwind-launch.exe"
    } else {
        "server/whirlwind-launch"
    };
    let manifest = json!({"manifest_version":"0.3","name":app.id,"display_name":app.title,"version":app.version,"description":app.description,"author":{"name":"Whirlwind"},"server":{"type":"binary","entry_point":entry,"mcp_config":{"command":format!("${{__dirname}}/{entry}"),"args":launch_args}},"tools_generated":true,"compatibility":{"platforms":[if cfg!(target_os="macos"){"darwin"}else if cfg!(windows){"win32"}else{"linux"}]}});
    let mut bytes = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut bytes);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(entry, options.unix_permissions(0o755))?;
        zip.write_all(crate::LAUNCHER)?;
        zip.start_file("manifest.json", options)?;
        zip.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
        zip.start_file(format!("skills/{}/SKILL.md", app.id), options)?;
        zip.write_all(app.skill.as_bytes())?;
        zip.finish()?;
    }
    atomic_write(path, &bytes.into_inner())
}
/// Connected means this host's native bridge has successfully reached the current app process.
pub fn verified_status(app: &App, h: &Homes, host: Host, endpoint_dir: &Path) -> Connection {
    let mut status = status(app, h, host);
    if status.state == "restart_required"
        && let Some(endpoint) = crate::endpoint::read(endpoint_dir)
        && let Ok(bytes) = fs::read(endpoint_dir.join(format!("client-{}.json", host.key())))
        && let Ok(stamp) = serde_json::from_slice::<Value>(&bytes)
        && stamp["pid"].as_u64() == Some(u64::from(endpoint.pid))
        && stamp["port"].as_u64() == Some(u64::from(endpoint.port))
    {
        status.state = "connected".into();
        status.detail = "The assistant has reached this app successfully.".into();
    }
    status
}
/// Explicit troubleshooting probe; sends only tools/list to the authenticated local app.
pub fn test(endpoint_dir: &Path) -> Result<()> {
    let endpoint =
        crate::endpoint::read(endpoint_dir).context("Enable AI assistants in this app first")?;
    // Tauri may call a synchronous command on a Tokio worker. Own the probe's
    // runtime on a separate thread so Test never nests block_on in that runtime.
    std::thread::Builder::new()
        .name("mcp-connection-test".into())
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(async {
                    let response = crate::http::Client::new()
                        .post(
                            endpoint.port,
                            &endpoint.token,
                            json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}})
                                .to_string(),
                            Duration::from_secs(5),
                        )
                        .await
                        .map_err(|_| {
                            anyhow::anyhow!("The local app did not answer the connection test")
                        })?;
                    ensure!(
                        response.status == 200,
                        "The app refused the connection test"
                    );
                    let reply: Value = serde_json::from_str(&response.body)?;
                    ensure!(
                        reply["result"]["tools"].is_array(),
                        "The app did not return its tool catalog"
                    );
                    Ok(())
                })
        })?
        .join()
        .map_err(|_| anyhow::anyhow!("The connection test worker failed"))?
}

fn same_path(a: &Path, b: &Path) -> bool {
    let canonical = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_owned());
    canonical(a) == canonical(b)
}
fn named_objects<'a>(value: &'a Value, name: &str, out: &mut Vec<&'a Value>) {
    match value {
        Value::Object(map) => {
            if map.get("name").and_then(Value::as_str) == Some(name) {
                out.push(value);
            }
            for child in map.values() {
                named_objects(child, name, out);
            }
        }
        Value::Array(values) => {
            for child in values {
                named_objects(child, name, out);
            }
        }
        _ => {}
    }
}
fn legacy_claude_entry(app: &App, h: &Homes, legacy_url: Option<&str>) -> Result<bool> {
    let candidates = if h.claude == h.home.join(".claude") {
        vec![h.home.join(".claude.json"), h.claude.join(".claude.json")]
    } else {
        vec![h.claude.join(".claude.json")]
    };
    for path in candidates {
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let value: Value = serde_json::from_slice(&bytes)
            .context("Claude's legacy registration could not be inspected")?;
        if let Some(entry) = value.get("mcpServers").and_then(|s| s.get(&app.server)) {
            ensure!(
                entry["command"].as_str() == app.executable.to_str()
                    || legacy_url.is_some_and(|url| entry["url"].as_str() == Some(url)),
                "A legacy Claude MCP entry has the same name but does not identify this app. It was left unchanged."
            );
            return Ok(true);
        }
    }
    Ok(false)
}

/// Read host-owned registration files without running CLIs every time Settings renders.
fn host_registration(app: &App, h: &Homes, host: Host) -> bool {
    let plugin = format!("{}@whirlwind", app.id);
    match host {
        Host::Codex => {
            let Ok(text) = fs::read_to_string(h.codex.join("config.toml")) else {
                return false;
            };
            let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
                return false;
            };
            doc.get("plugins")
                .and_then(|p| p.get(&plugin))
                .and_then(|p| p.get("enabled"))
                .and_then(|v| v.as_bool())
                == Some(true)
                && h.codex
                    .join("plugins/cache/whirlwind")
                    .join(&app.id)
                    .join(&app.version)
                    .join("mcp.json")
                    .is_file()
        }
        Host::ClaudeCode => {
            let read = |path: PathBuf| {
                fs::read(path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            };
            let Some(settings) = read(h.claude.join("settings.json")) else {
                return false;
            };
            let Some(installed) = read(h.claude.join("plugins/installed_plugins.json")) else {
                return false;
            };
            settings["enabledPlugins"][&plugin] == true
                && installed["plugins"][&plugin]
                    .as_array()
                    .is_some_and(|entries| {
                        entries.iter().any(|entry| {
                            entry["scope"] == "user"
                                && entry["installPath"]
                                    .as_str()
                                    .is_some_and(|p| Path::new(p).is_dir())
                        })
                    })
        }
        Host::ClaudeDesktop => true, // Completion is verified only by a successful bridge call.
    }
}

fn contains_plugin(value: &Value, id: &str) -> bool {
    match value {
        Value::Object(map) => {
            map.get("pluginId")
                .or_else(|| map.get("id"))
                .and_then(Value::as_str)
                == Some(id)
                || map.values().any(|v| contains_plugin(v, id))
        }
        Value::Array(values) => values.iter().any(|v| contains_plugin(v, id)),
        _ => false,
    }
}
/// Only migrate the legacy Hindsight Sync plugin when the marketplace points into
/// this installed application's resource directory. Same-name foreign sources stay intact.
fn old_sync_marketplace(app: &App, host: Host, listing: &Value) -> Result<bool> {
    if app.id != "hindsight-sync" {
        return Ok(false);
    }
    let mut matches = Vec::new();
    named_objects(listing, "hindsight-sync", &mut matches);
    if matches.is_empty() {
        return Ok(false);
    }
    ensure!(
        matches.len() == 1,
        "Ambiguous legacy Hindsight Sync marketplace; remove its duplicate in the assistant first."
    );
    let entry = matches[0];
    let path = if host == Host::ClaudeCode {
        entry["path"].as_str()
    } else {
        entry["root"].as_str()
    }
    .map(Path::new);
    let suffix = Path::new("plugins").join(if host == Host::Codex {
        "codex"
    } else {
        "claude-code"
    });
    let parent = app
        .executable
        .parent()
        .context("App executable has no parent")?;
    let mut candidates = vec![parent.join(&suffix), parent.join("resources").join(&suffix)];
    if let Some(contents) = parent.parent() {
        candidates.push(contents.join("Resources").join(&suffix));
    }
    if cfg!(target_os = "linux") {
        candidates.push(Path::new("/usr/lib/Hindsight Sync").join(&suffix));
    }
    ensure!(
        path.is_some_and(|path| candidates
            .iter()
            .any(|candidate| same_path(path, candidate))),
        "The legacy Hindsight Sync marketplace points to another location. Remove that legacy plugin in the assistant before connecting; it was left unchanged."
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(dir: &Path) -> (App, Homes) {
        let h = Homes::isolated(dir.into());
        let app = App {
            id: "test-app".into(),
            server: "test".into(),
            title: "Test".into(),
            version: "1.0.0".into(),
            description: "Test native app".into(),
            executable: PathBuf::from("/Applications/Test.app/Contents/MacOS/test"),
            args: vec!["--mcp".into()],
            skill: "---\nname: test-app\ndescription: Test\n---\nUse the guide.".into(),
            tools: vec![
                ("read".into(), Effect::Read),
                ("export".into(), Effect::Files),
                ("delete".into(), Effect::Discard),
            ],
        };
        (app, h)
    }
    #[tokio::test]
    async fn connection_test_is_safe_inside_an_async_command() {
        use std::io::{Read, Write};
        let dir = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        crate::endpoint::write(
            dir.path(),
            &crate::endpoint::Endpoint {
                version: 1,
                port: listener.local_addr().unwrap().port(),
                token: "probe".into(),
                pid: std::process::id(),
            },
        )
        .unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 4096];
            let count = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..count]).contains("tools/list"));
            let body = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        });
        test(dir.path()).unwrap();
        server.join().unwrap();
    }
    #[test]
    fn packages_have_native_launchers_and_no_secrets() {
        let d = tempfile::tempdir().unwrap();
        let (app, h) = fixture(d.path());
        let p = package(&app, &h).unwrap();
        let m: Value = serde_json::from_slice(&fs::read(p.join("mcp.json")).unwrap()).unwrap();
        assert_eq!(m["mcpServers"]["test"]["args"][1], "--mcp");
        assert!(p.join("skills/test-app/SKILL.md").exists());
        assert!(
            h.marketplace
                .join(".claude-plugin/marketplace.json")
                .exists()
        );
    }
    #[test]
    fn policy_preserves_user_deny_and_unrelated_configuration() {
        let d = tempfile::tempdir().unwrap();
        let (app, h) = fixture(d.path());
        atomic_write(&h.codex.join("config.toml"),b"# keep me\nmodel = 'test'\n[plugins.'test-app@whirlwind'.mcp_servers.test.tools.export]\napproval_mode = 'prompt'\n").unwrap();
        let approvals =
            codex_policy(&app, &h, Policy::Workflow, &Receipt::default(), None).unwrap();
        let text = fs::read_to_string(h.codex.join("config.toml")).unwrap();
        assert!(text.contains("# keep me"));
        assert!(!approvals.contains_key("export"));
        assert_eq!(approvals["read"], "approve");
        assert_eq!(approvals["delete"], "prompt");
        atomic_write(&h.claude.join("settings.json"),br#"{"permissions":{"allow":["user-rule"],"deny":["mcp__plugin_test-app_test__read"]},"theme":"dark"}"#).unwrap();
        let rules = claude_policy(&app, &h, Policy::Workflow, &Receipt::default()).unwrap();
        let before = Receipt {
            rules,
            ..Default::default()
        };
        claude_policy(&app, &h, Policy::AlwaysAsk, &before).unwrap();
        let v: Value =
            serde_json::from_slice(&fs::read(h.claude.join("settings.json")).unwrap()).unwrap();
        assert_eq!(v["permissions"]["allow"], json!(["user-rule"]));
        assert_eq!(v["permissions"]["deny"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn malformed_configuration_is_not_replaced() {
        let d = tempfile::tempdir().unwrap();
        let (app, h) = fixture(d.path());
        atomic_write(&h.codex.join("config.toml"), b"invalid [").unwrap();
        assert!(codex_policy(&app, &h, Policy::Workflow, &Receipt::default(), None).is_err());
        assert_eq!(fs::read(h.codex.join("config.toml")).unwrap(), b"invalid [");
    }
}
