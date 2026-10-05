//! Claude Code's permission rules for the service (spec.md 8.8).
//!
//! Claude Code asks before every MCP tool unless a rule in a settings file
//! allows it, and it does not act on the tools' annotations
//! (`tools::effects`), so *Add to Claude Code* writes the rules the person's
//! choice implies into `~/.claude/settings.json` — user scope, as the server
//! itself is registered. No command line adds a permission rule, and unlike
//! `~/.claude.json` this file is a documented one the person edits by hand,
//! so it is edited here, keeping every key we do not own.
//!
//! What we own is exactly the rules the last registration wrote, remembered
//! in our own settings (`McpSettings::claude_rules`): a rule the person wrote
//! themselves, even one naming a VectorEffects tool, is never removed.

use std::path::Path;

use crate::error::{AppError, Context, Result};
use crate::settings::McpAsk;

use super::clients::SERVER_NAME;
use super::tools::effects::{TABLE, runs_without_asking};

/// The allow rules `ask` implies. Enumerated rather than a wildcard with
/// exceptions, so a tool added in a later version asks until it has been
/// classified and the person has registered again.
pub fn rules_for(ask: McpAsk) -> Vec<String> {
    match ask {
        McpAsk::Everything => Vec::new(),
        McpAsk::Nothing => vec![format!("mcp__{SERVER_NAME}__*")],
        McpAsk::Outside => TABLE
            .iter()
            .filter(|&&(_, effect)| runs_without_asking(effect, ask))
            .map(|(name, _)| format!("mcp__{SERVER_NAME}__{name}"))
            .collect(),
    }
}

fn refused(why: impl Into<String>) -> AppError {
    AppError::Doing {
        doing: "add the MCP service to",
        what: "Claude Code's settings.json".to_owned(),
        why: why.into(),
    }
}

/// An edited `settings.json`, and which rules the edit put there.
#[derive(Debug)]
pub struct Edited {
    /// The file's new text.
    pub text: String,
    /// The rules this edit inserted. A rule that was there already is the
    /// person's, not ours, and is not in this list — so it is not removed
    /// when the choice later changes.
    pub written: Vec<String>,
}

/// `settings.json`'s text with `previous` taken out of `permissions.allow`
/// and `rules` put in, and everything else as it was. `None` is a file that
/// is not there yet.
///
/// A file that is not a JSON object, or whose `permissions` or `allow` is the
/// wrong type, is refused rather than repaired: it is the person's file.
pub fn with_rules(existing: Option<&str>, previous: &[String], rules: &[String]) -> Result<Edited> {
    let mut document: serde_json::Value = match existing {
        Some(text) => serde_json::from_str(text).map_err(|err| {
            refused(format!(
                "it is not valid JSON ({err}), so it was left as it is"
            ))
        })?,
        None => serde_json::json!({}),
    };
    let root = document
        .as_object_mut()
        .ok_or_else(|| refused("it is not a JSON object, so it was left as it is"))?;
    let permissions = root
        .entry("permissions")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| refused("its `permissions` is not an object"))?;
    let allow = permissions
        .entry("allow")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| refused("its `permissions.allow` is not a list"))?;
    allow.retain(|rule| {
        !rule
            .as_str()
            .is_some_and(|r| previous.iter().any(|p| p == r))
    });
    let mut written = Vec::new();
    for rule in rules {
        if !allow.iter().any(|r| r.as_str() == Some(rule)) {
            allow.push(serde_json::Value::String(rule.clone()));
            written.push(rule.clone());
        }
    }
    let mut text = serde_json::to_string_pretty(&document)
        .map_err(|err| AppError::Internal(err.to_string()))?;
    text.push('\n');
    Ok(Edited { text, written })
}

/// Rewrites `<claude_home>/settings.json` with the service's rules.
///
/// `remember` keeps our record of which rules are ours (`McpSettings::
/// claude_rules`). It is called **before** the file is written, with every
/// rule that may be on disk afterwards — the previous ones and the new — so
/// a failure between the two leaves a record that over-claims rather than
/// one that forgets a rule we wrote; taking out a rule that is not there is
/// nothing. The caller then records exactly what was written.
///
/// A `settings.json` that is a symbolic link — dotfile managers link this
/// file into a repository — is written through: the file it points at is
/// replaced, and the link stays a link.
pub fn write_rules(
    claude_home: &Path,
    previous: &[String],
    rules: &[String],
    mut remember: impl FnMut(&[String]) -> Result<()>,
) -> Result<Vec<String>> {
    std::fs::create_dir_all(claude_home).doing("create", claude_home.display())?;
    let link = claude_home.join("settings.json");
    let file = match std::fs::canonicalize(&link) {
        Ok(target) => target,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => link,
        Err(err) => return Err(err).doing("read", link.display()),
    };
    let existing = match std::fs::read_to_string(&file) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err).doing("read", file.display()),
    };
    let edited = with_rules(existing.as_deref(), previous, rules)?;

    let mut either: Vec<String> = previous.to_vec();
    either.extend(
        edited
            .written
            .iter()
            .filter(|r| !previous.contains(r))
            .cloned(),
    );
    remember(&either)?;

    // Beside the file and then renamed over it, so a failure half way leaves
    // the person's configuration as it was rather than truncated. The file
    // holds no secret, so it keeps whatever mode it had.
    let folder = file.parent().unwrap_or(claude_home);
    let staged = folder.join(".settings.json.vectoreffects");
    let written = (|| {
        std::fs::write(&staged, &edited.text).doing("write", staged.display())?;
        if let Ok(meta) = std::fs::metadata(&file) {
            std::fs::set_permissions(&staged, meta.permissions())
                .doing("restrict", staged.display())?;
        }
        std::fs::rename(&staged, &file).doing("write", file.display())
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    written?;
    Ok(edited.written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::McpAsk;

    /// A scratch folder, removed when dropped.
    struct TempRoot(std::path::PathBuf);

    impl TempRoot {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("ve-claude-settings-{}-{label}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("temp root");
            Self(dir)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_missing_settings_file_is_created() {
        let out = with_rules(None, &[], &rules_for(McpAsk::Outside))
            .expect("edit")
            .text;
        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        let allow = v["permissions"]["allow"].as_array().expect("allow");
        assert!(allow.iter().any(|r| r == "mcp__vectoreffects__layers_list"));
        assert!(!allow.iter().any(|r| r == "mcp__vectoreffects__invoke"));
        assert!(!allow.iter().any(|r| r == "mcp__vectoreffects__export_grib"));
    }

    #[test]
    fn a_missing_folder_and_file_are_created_on_disk() {
        let root = TempRoot::new("fresh");
        let home = root.0.join(".claude");
        write_rules(&home, &[], &rules_for(McpAsk::Nothing), |_| Ok(())).expect("write");
        let text = std::fs::read_to_string(home.join("settings.json")).expect("written");
        let v: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(
            v["permissions"]["allow"],
            serde_json::json!(["mcp__vectoreffects__*"])
        );
    }

    #[test]
    fn everything_else_in_the_file_is_kept_in_order() {
        let before = "{\n  \"model\": \"opus\",\n  \"permissions\": {\n    \"deny\": [\"Bash(rm *)\"],\n    \"allow\": [\"Bash(git status)\"]\n  },\n  \"theme\": \"dark\"\n}\n";
        let out = with_rules(Some(before), &[], &rules_for(McpAsk::Nothing))
            .expect("edit")
            .text;
        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["model"], "opus");
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["permissions"]["deny"][0], "Bash(rm *)");
        assert_eq!(
            v["permissions"]["allow"],
            serde_json::json!(["Bash(git status)", "mcp__vectoreffects__*"])
        );
        let keys: Vec<&str> = v
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            ["model", "permissions", "theme"],
            "preserve_order must be on"
        );
    }

    #[test]
    fn rules_the_person_wrote_survive_a_reregistration() {
        let ours = rules_for(McpAsk::Nothing);
        // Last time we wrote the wildcard; the person added export_grib by hand.
        let before = r#"{"permissions":{"allow":["mcp__vectoreffects__export_grib","mcp__vectoreffects__*"]}}"#;
        let out = with_rules(Some(before), &ours, &rules_for(McpAsk::Everything))
            .expect("edit")
            .text;
        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(
            v["permissions"]["allow"],
            serde_json::json!(["mcp__vectoreffects__export_grib"])
        );
    }

    #[test]
    fn a_rule_already_there_is_not_written_twice() {
        let before = r#"{"permissions":{"allow":["mcp__vectoreffects__*"]}}"#;
        let out = with_rules(Some(before), &[], &rules_for(McpAsk::Nothing))
            .expect("edit")
            .text;
        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(
            v["permissions"]["allow"],
            serde_json::json!(["mcp__vectoreffects__*"])
        );
    }

    #[test]
    fn a_rule_the_person_already_had_is_not_taken_as_ours() {
        // The person allowed layers_list by hand; the default choice also
        // wants it. It must not be remembered as written by us, or the next
        // registration (ask before everything) would remove it.
        let before = r#"{"permissions":{"allow":["mcp__vectoreffects__layers_list"]}}"#;
        let edit = with_rules(Some(before), &[], &rules_for(McpAsk::Outside)).expect("edit");
        assert!(
            !edit
                .written
                .iter()
                .any(|r| r == "mcp__vectoreffects__layers_list")
        );
        assert_eq!(edit.written.len(), 44);
        let again = with_rules(
            Some(&edit.text),
            &edit.written,
            &rules_for(McpAsk::Everything),
        )
        .expect("edit");
        let v: serde_json::Value = serde_json::from_str(&again.text).expect("json");
        assert_eq!(
            v["permissions"]["allow"],
            serde_json::json!(["mcp__vectoreffects__layers_list"])
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_settings_file_that_is_a_link_stays_a_link() {
        // Dotfile managers link this exact file into a repository.
        let root = TempRoot::new("link");
        let real = root.0.join("dotfiles");
        std::fs::create_dir_all(&real).expect("dotfiles");
        std::fs::write(real.join("settings.json"), "{\"theme\":\"dark\"}\n").expect("real");
        let home = root.0.join(".claude");
        std::fs::create_dir_all(&home).expect("home");
        std::os::unix::fs::symlink(real.join("settings.json"), home.join("settings.json"))
            .expect("link");
        write_rules(&home, &[], &rules_for(McpAsk::Nothing), |_| Ok(())).expect("write");
        let meta = std::fs::symlink_metadata(home.join("settings.json")).expect("meta");
        assert!(
            meta.file_type().is_symlink(),
            "the link was replaced by a file"
        );
        let text = std::fs::read_to_string(real.join("settings.json")).expect("read");
        assert!(text.contains("mcp__vectoreffects__*"), "{text}");
        assert!(text.contains("dark"), "{text}");
    }

    #[cfg(unix)]
    #[test]
    fn rules_are_remembered_before_the_file_is_written() {
        // If the file cannot be written after our record is saved, the record
        // must still name everything that may be on disk — the old rules and
        // the new — so a later registration can take them out.
        use std::os::unix::fs::PermissionsExt;
        let root = TempRoot::new("remember");
        let home = root.0.join(".claude");
        std::fs::create_dir_all(&home).expect("home");
        let previous = vec!["mcp__vectoreffects__*".to_owned()];
        std::fs::write(
            home.join("settings.json"),
            "{\"permissions\":{\"allow\":[\"mcp__vectoreffects__*\"]}}\n",
        )
        .expect("seed");
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o500)).expect("ro");
        let mut remembered = Vec::new();
        let result = write_rules(&home, &previous, &rules_for(McpAsk::Outside), |rules| {
            remembered.push(rules.to_vec());
            Ok(())
        });
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).expect("rw");
        assert!(
            result.is_err(),
            "a read-only folder cannot take the staged file"
        );
        let first = remembered.first().expect("remembered before writing");
        assert!(first.contains(&"mcp__vectoreffects__*".to_owned()));
        assert!(first.contains(&"mcp__vectoreffects__layers_list".to_owned()));
        assert_eq!(
            remembered.len(),
            1,
            "nothing remembered after a failed write"
        );
    }

    #[test]
    fn an_unparseable_settings_file_is_refused_and_untouched() {
        let root = TempRoot::new("unparseable");
        let file = root.0.join("settings.json");
        let text = "{ \"model\": \"opus\", }\n"; // a trailing comma
        std::fs::write(&file, text).expect("write");
        assert!(write_rules(&root.0, &[], &rules_for(McpAsk::Outside), |_| Ok(())).is_err());
        assert_eq!(std::fs::read_to_string(&file).expect("read"), text);
    }

    #[test]
    fn a_permissions_value_of_the_wrong_type_is_refused() {
        let rules = rules_for(McpAsk::Outside);
        assert!(with_rules(Some(r#"{"permissions":[]}"#), &[], &rules).is_err());
        assert!(with_rules(Some(r#"{"permissions":{"allow":"x"}}"#), &[], &rules).is_err());
        assert!(with_rules(Some("[]"), &[], &rules).is_err());
    }

    #[test]
    fn every_rule_is_one_claude_code_accepts() {
        // An allow rule needs a literal `mcp__<server>__` prefix; the bare
        // server name is not valid there (code.claude.com/docs/en/permissions).
        for ask in [McpAsk::Outside, McpAsk::Nothing] {
            for rule in rules_for(ask) {
                assert!(rule.starts_with("mcp__vectoreffects__"), "{rule}");
            }
        }
        assert!(rules_for(McpAsk::Everything).is_empty());
        assert_eq!(rules_for(McpAsk::Outside).len(), 45);
    }
}
