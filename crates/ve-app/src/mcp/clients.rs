//! Registering the service with a client on this computer (spec.md 8.8):
//! the Settings dialog's "Add to Claude Code" and "Add to Codex".
//!
//! The two clients are registered differently, and on purpose. Claude Code
//! keeps its servers in `~/.claude.json`, which is also its live state and
//! is rewritten by every running session, so the only safe writer is Claude
//! Code itself: this runs `claude mcp add`. Codex's `codex mcp add` cannot
//! carry a token — it takes the *name of an environment variable* and
//! nothing else — but its `~/.codex/config.toml` is a documented file the
//! person edits by hand, so that one is edited here, keeping every other
//! line as it was.
//!
//! Nothing here touches the network or opens a socket (invariant 5). It does
//! start a process and write outside the application's own folders, which is
//! why it happens on a click and never as a side effect of another setting.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::commands::AppState;
use crate::error::{AppError, Context, Result};
use crate::projects::with_session;

/// The name the service is registered under, in every client.
pub const SERVER_NAME: &str = "vectoreffects";

/// A client the Settings dialog can register the service with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "McpClient.ts")]
pub enum McpClient {
    ClaudeCode,
    Codex,
    /// Not a configuration entry but an extension: see [`super::desktop`].
    ClaudeDesktop,
}

impl McpClient {
    /// The clients this platform has, in the order the dialog offers them.
    /// Claude Desktop does not exist on Linux, and a button that can only
    /// refuse is worse than no button.
    pub fn available() -> Vec<Self> {
        let mut clients = vec![Self::ClaudeCode, Self::Codex];
        if super::desktop::supported() {
            clients.push(Self::ClaudeDesktop);
        }
        clients
    }
}

/// The arguments of `claude mcp remove` and `claude mcp add`, in that order.
///
/// Two calls because `add` refuses a name that exists, and the button has to
/// work a second time: after a rotated token it is how the client is
/// brought up to date. User scope, so the server is there whichever folder
/// Claude Code is started in; the default, local, would bind it to the
/// application's own working directory.
pub fn claude_code_args(url: &str, token: &str) -> [Vec<String>; 2] {
    let owned = |args: &[&str]| args.iter().map(|&a| a.to_owned()).collect();
    [
        owned(&["mcp", "remove", "--scope", "user", SERVER_NAME]),
        owned(&[
            "mcp",
            "add",
            "--scope",
            "user",
            "--transport",
            "http",
            SERVER_NAME,
            url,
            "--header",
            &format!("Authorization: Bearer {token}"),
        ]),
    ]
}

/// Where a command-line client is installed, beyond what `PATH` says.
///
/// An application started from the Finder or the Start menu does not get
/// the shell's `PATH`, so `claude` typed in a terminal and `claude` looked
/// up from here are different questions. These are the places the two
/// installers and the package managers put it.
fn install_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![
        home.join(".local").join("bin"),
        home.join(".claude").join("local"),
    ];
    if cfg!(windows) {
        dirs.push(home.join("AppData").join("Roaming").join("npm"));
    } else {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(home.join(".npm-global").join("bin"));
    }
    dirs
}

/// The directories to look in, the known ones first and then `PATH`.
fn search_dirs(home: &Path, path_var: Option<&OsStr>) -> Vec<PathBuf> {
    let mut dirs = install_dirs(home);
    if let Some(path_var) = path_var {
        dirs.extend(std::env::split_paths(path_var));
    }
    dirs
}

/// Finds `binary` in the known install places and then on `PATH`.
pub fn locate(binary: &str, home: &Path, path_var: Option<&OsStr>) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{binary}.exe"), format!("{binary}.cmd")]
    } else {
        vec![binary.to_owned()]
    };
    search_dirs(home, path_var)
        .into_iter()
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}

/// Registers the service with Claude Code through its own command line.
///
/// `child_path` becomes the child's `PATH`: an npm-installed `claude` is a
/// script that starts `node`, and would not find it on the bare `PATH` a
/// windowed application inherits.
pub fn register_claude_code(
    claude: &Path,
    child_path: &OsStr,
    url: &str,
    token: &str,
) -> Result<()> {
    let [remove, add] = claude_code_args(url, token);
    // Removing a name that is not there fails, and that is the usual case.
    let _ = run(claude, child_path, &remove);
    let output = run(claude, child_path, &add).doing("run", claude.display())?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    let said = if said.trim().is_empty() {
        String::from_utf8_lossy(&output.stdout)
    } else {
        said
    };
    Err(AppError::Doing {
        doing: "add the MCP service to",
        what: "Claude Code".to_owned(),
        // The command line carries the token; what the client printed does
        // not (it prints headers redacted), so only that is repeated.
        why: said.trim().to_owned(),
    })
}

fn run(
    program: &Path,
    child_path: &OsStr,
    args: &[String],
) -> std::io::Result<std::process::Output> {
    let mut command = Command::new(program);
    command
        .args(args)
        .env("PATH", child_path)
        .stdin(std::process::Stdio::null());
    // A console program started from a windowed one flashes a console.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.output()
}

/// `config.toml` with the service's entry set, and every other line kept.
///
/// An entry that is already there is updated in place rather than replaced:
/// whatever else the person put on it (`enabled`, a timeout) is theirs. A
/// `bearer_token_env_var` is dropped, since Codex would send that token
/// rather than the header's.
pub fn codex_config_with_entry(existing: &str, url: &str, token: &str) -> Result<String> {
    let not_a_table = |what: &str| AppError::Doing {
        doing: "add the MCP service to",
        what: "Codex's config.toml".to_owned(),
        why: format!("`{what}` is there already and is not a table"),
    };
    let mut document: toml_edit::DocumentMut =
        existing.parse().doing("read", "Codex's config.toml")?;
    let servers = document
        .entry("mcp_servers")
        .or_insert_with(|| {
            let mut table = toml_edit::Table::new();
            // `[mcp_servers.vectoreffects]` alone, not an empty
            // `[mcp_servers]` header above it.
            table.set_implicit(true);
            toml_edit::Item::Table(table)
        })
        .as_table_mut()
        .ok_or_else(|| not_a_table("mcp_servers"))?;
    let entry = servers
        .entry(SERVER_NAME)
        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| not_a_table("mcp_servers.vectoreffects"))?;
    entry.insert("url", toml_edit::value(url));
    let mut headers = toml_edit::InlineTable::new();
    headers.insert("Authorization", format!("Bearer {token}").into());
    entry.insert("http_headers", toml_edit::value(headers));
    entry.remove("bearer_token_env_var");
    Ok(document.to_string())
}

/// Registers the service in the `config.toml` under `codex_home`.
///
/// Refused when the folder is not there: that is Codex not being installed,
/// and making the folder would only hide it.
pub fn register_codex(codex_home: &Path, url: &str, token: &str) -> Result<()> {
    if !codex_home.is_dir() {
        return Err(AppError::Doing {
            doing: "add the MCP service to",
            what: "Codex".to_owned(),
            why: format!(
                "there is no {} on this computer, so Codex does not seem to be installed",
                codex_home.display()
            ),
        });
    }
    let file = codex_home.join("config.toml");
    let existing = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err).doing("read", file.display()),
    };
    let updated = codex_config_with_entry(&existing, url, token)?;
    // Beside the file and then renamed over it, so a failure half way leaves
    // the person's configuration as it was rather than truncated.
    let staged = codex_home.join(".config.toml.vectoreffects");
    std::fs::write(&staged, updated).doing("write", staged.display())?;
    // The file now holds a bearer token: owner-only, as the settings file is.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600))
            .doing("restrict", staged.display())?;
    }
    std::fs::rename(&staged, &file).doing("write", file.display())
}

/// Where Codex keeps its configuration: `$CODEX_HOME`, or `~/.codex`.
pub fn codex_home(home: &Path, codex_home_var: Option<OsString>) -> PathBuf {
    codex_home_var
        .filter(|value| !value.is_empty())
        .map_or_else(|| home.join(".codex"), PathBuf::from)
}

/// Adds the service to a client's own configuration, or brings an entry
/// already there up to date with the current port and token.
///
/// `async`: it starts a process and waits for it.
#[tauri::command(async)]
pub fn mcp_register_client(state: tauri::State<'_, AppState>, client: McpClient) -> Result<()> {
    let mcp = with_session(&state, |session| Ok(session.settings.mcp.clone()))?;
    if !mcp.enabled || mcp.token.is_empty() {
        return Err(AppError::BadOption {
            field: "mcp",
            value: "is not turned on".to_owned(),
        });
    }
    let url = format!("http://127.0.0.1:{}/mcp", mcp.port);
    let home = directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .ok_or_else(|| AppError::Internal("this account has no home folder".to_owned()))?;
    match client {
        McpClient::ClaudeCode => {
            let path_var = std::env::var_os("PATH");
            let claude = locate("claude", &home, path_var.as_deref()).ok_or_else(|| {
                AppError::Doing {
                    doing: "add the MCP service to",
                    what: "Claude Code".to_owned(),
                    why: "the `claude` command was not found on this computer. Copy the command below and run it in a terminal instead".to_owned(),
                }
            })?;
            let child_path = std::env::join_paths(search_dirs(&home, path_var.as_deref()))
                .map_err(|err| AppError::Internal(err.to_string()))?;
            register_claude_code(&claude, &child_path, &url, &mcp.token)
        }
        McpClient::ClaudeDesktop => super::desktop::register(&state.paths.settings_file()),
        McpClient::Codex => register_codex(
            &codex_home(&home, std::env::var_os("CODEX_HOME")),
            &url,
            &mcp.token,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://127.0.0.1:47391/mcp";

    /// The reference is Codex's own reader: `codex mcp get` on a file of this
    /// shape reports `transport: streamable_http` and an `Authorization`
    /// header (checked by hand against codex-cli 0.154.0). Here the file is
    /// read back by a TOML parser and the two values looked up.
    #[test]
    fn an_empty_codex_config_gains_the_entry() {
        let text = codex_config_with_entry("", URL, "tok_abc").expect("edit");
        assert!(text.contains("[mcp_servers.vectoreffects]"), "{text}");
        assert!(!text.contains("[mcp_servers]\n"), "{text}");
        let parsed: toml_edit::DocumentMut = text.parse().expect("valid toml");
        let entry = &parsed["mcp_servers"]["vectoreffects"];
        assert_eq!(entry["url"].as_str(), Some(URL));
        assert_eq!(
            entry["http_headers"]["Authorization"].as_str(),
            Some("Bearer tok_abc")
        );
    }

    #[test]
    fn a_codex_config_keeps_everything_that_is_not_the_entry() {
        let before = "# mine\nmodel = \"o3\"  # keep\n\n[mcp_servers.other]\ncommand = \"npx\"\n\n[mcp_servers.vectoreffects]\nurl = \"http://127.0.0.1:1/mcp\"\nenabled = false\nbearer_token_env_var = \"OLD\"\nhttp_headers = { Authorization = \"Bearer stale\" }\n\n[profiles.fast]\nmodel = \"mini\"\n";
        let after = codex_config_with_entry(before, URL, "tok_new").expect("edit");
        // Every line that is not the two values or the dropped variable.
        for kept in [
            "# mine",
            "model = \"o3\"  # keep",
            "[mcp_servers.other]",
            "command = \"npx\"",
            "enabled = false",
            "[profiles.fast]",
            "model = \"mini\"",
        ] {
            assert!(after.contains(kept), "lost {kept:?} from:\n{after}");
        }
        assert!(!after.contains("stale"), "{after}");
        assert!(!after.contains("bearer_token_env_var"), "{after}");
        assert!(after.contains("Bearer tok_new"), "{after}");
        assert_eq!(after.matches("[mcp_servers.vectoreffects]").count(), 1);
        // Doing it again changes nothing: the button is repeatable.
        let again = codex_config_with_entry(&after, URL, "tok_new").expect("edit");
        assert_eq!(again, after);
    }

    #[test]
    fn a_codex_config_that_is_not_toml_is_refused_and_left_alone() {
        let home = tempfile::tempdir().expect("tempdir");
        let file = home.path().join("config.toml");
        std::fs::write(&file, "model = [unclosed").expect("write");
        assert!(register_codex(home.path(), URL, "tok").is_err());
        let text = std::fs::read_to_string(&file).expect("read");
        assert_eq!(text, "model = [unclosed");
    }

    #[test]
    fn codex_is_refused_when_its_folder_is_missing() {
        let home = tempfile::tempdir().expect("tempdir");
        let missing = home.path().join(".codex");
        let err = register_codex(&missing, URL, "tok").expect_err("no folder");
        assert!(err.to_string().contains("does not seem to be installed"));
        assert!(!missing.exists(), "the folder must not be made");
    }

    #[cfg(unix)]
    #[test]
    fn the_codex_config_is_written_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().expect("tempdir");
        register_codex(home.path(), URL, "tok").expect("register");
        let file = home.path().join("config.toml");
        let mode = std::fs::metadata(&file)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "config.toml mode was {mode:o}");
        let names: Vec<_> = std::fs::read_dir(home.path())
            .expect("read_dir")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(names, [OsString::from("config.toml")], "staging file left");
    }

    #[test]
    fn codex_home_prefers_the_variable_and_ignores_an_empty_one() {
        let home = Path::new("/home/someone");
        assert_eq!(codex_home(home, None), home.join(".codex"));
        assert_eq!(codex_home(home, Some(OsString::new())), home.join(".codex"));
        assert_eq!(
            codex_home(home, Some(OsString::from("/elsewhere"))),
            PathBuf::from("/elsewhere")
        );
    }

    #[cfg(unix)]
    #[test]
    fn locate_finds_an_install_the_path_does_not_name() {
        let home = tempfile::tempdir().expect("tempdir");
        let bin = home.path().join(".local").join("bin");
        std::fs::create_dir_all(&bin).expect("mkdir");
        std::fs::write(bin.join("claude-ve-test"), "").expect("write");
        // A directory of the same name is not an install.
        std::fs::create_dir_all(home.path().join(".claude/local/claude-ve-dir")).expect("mkdir");
        let empty_path = OsString::from("/nonexistent-ve");
        assert_eq!(
            locate("claude-ve-test", home.path(), Some(&empty_path)),
            Some(bin.join("claude-ve-test"))
        );
        assert_eq!(
            locate("claude-ve-dir", home.path(), Some(&empty_path)),
            None
        );
        // And `PATH` is searched when the known places have nothing.
        let other = tempfile::tempdir().expect("tempdir");
        std::fs::write(other.path().join("claude-ve-path"), "").expect("write");
        assert_eq!(
            locate(
                "claude-ve-path",
                home.path(),
                Some(other.path().as_os_str())
            ),
            Some(other.path().join("claude-ve-path"))
        );
    }

    /// A stand-in `claude` that records what it was started with, so the
    /// test sees the arguments as a process receives them — one per line,
    /// the header's space and colon intact — not as this module built them.
    #[cfg(unix)]
    fn fake_claude(dir: &Path, add_exit: i32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let log = dir.join("calls.log");
        let script = dir.join("claude");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\" >> '{log}'; done\nprintf -- '--\\n' >> '{log}'\nif [ \"$2\" = remove ]; then exit 1; fi\necho 'it said why' >&2\nexit {add_exit}\n",
                log = log.display()
            ),
        )
        .expect("write script");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        script
    }

    #[cfg(unix)]
    #[test]
    fn claude_code_is_asked_to_remove_and_then_add_at_user_scope() {
        let dir = tempfile::tempdir().expect("tempdir");
        let claude = fake_claude(dir.path(), 0);
        // The failing `remove` (nothing registered yet) must not stop the add.
        register_claude_code(&claude, OsStr::new("/bin:/usr/bin"), URL, "tok_abc").expect("add");
        let calls = std::fs::read_to_string(dir.path().join("calls.log")).expect("log");
        assert_eq!(
            calls,
            "mcp\nremove\n--scope\nuser\nvectoreffects\n--\n\
             mcp\nadd\n--scope\nuser\n--transport\nhttp\nvectoreffects\n\
             http://127.0.0.1:47391/mcp\n--header\nAuthorization: Bearer tok_abc\n--\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_refusal_from_claude_code_is_reported_in_its_own_words_without_the_token() {
        let dir = tempfile::tempdir().expect("tempdir");
        let claude = fake_claude(dir.path(), 3);
        let err = register_claude_code(&claude, OsStr::new("/bin:/usr/bin"), URL, "tok_secret")
            .expect_err("exit 3");
        let message = err.to_string();
        assert!(message.contains("it said why"), "{message}");
        assert!(!message.contains("tok_secret"), "{message}");
    }
}
