# Fewer Permission Prompts for the MCP Service — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An agent driving VectorEffects through its MCP service stops asking
the person's permission for anything that stays inside the application and can
be undone, and still asks before anything that writes files, downloads,
discards unsaved work or runs an arbitrary command — in Claude Code, Codex and
(as far as it allows) Claude Desktop.

**Architecture:** One table in `ve-app` classifies every MCP tool by its
*effect*. The server publishes that classification as the standard MCP tool
annotations (`readOnlyHint`, `destructiveHint`, `openWorldHint`,
`idempotentHint`), which Codex already acts on. Claude Code ignores
annotations, so *Add to Claude Code* additionally writes `permissions.allow`
rules for the chosen effects into `~/.claude/settings.json`, and *Add to
Codex* writes `default_tools_approval_mode` for the choices annotations cannot
express. Which effects run without asking is one new setting, offered beside
the Add buttons.

**Tech Stack:** Rust (`rmcp` 3.4 tool annotations, `serde_json` with
`preserve_order`, `toml_edit`), React/TypeScript settings dialog, the nine
interface languages.

**Spec:** `spec.md` §8.8 (MCP service), and the research recorded in
*Background* below. The spec changes in Task 6.

## Background: what each client does (researched 2026-10-04)

| Client | What decides a prompt | Source |
|---|---|---|
| Claude Code | Only `permissions` rules. `mcp__vectoreffects__<tool>` allows one tool; `mcp__vectoreffects__*` all of them (the bare `mcp__vectoreffects` is **not** valid in an allow rule). Precedence deny → ask → allow. Annotations are not documented to affect prompting. A skill's `allowed-tools` lasts one turn. No CLI command adds a permission rule. | code.claude.com/docs/en/permissions, /settings, /skills |
| Codex (0.155) | `[mcp_servers.<id>] default_tools_approval_mode` and `[mcp_servers.<id>.tools.<t>] approval_mode`: `auto` (default), `prompt`, `writes`, `approve`. Under `auto`: `destructiveHint=true` asks; else `readOnlyHint=true` runs; else it runs only when **both** `destructiveHint=false` and `openWorldHint=false`. A missing hint counts as `true`, so today every one of our tools prompts. "Always allow" persists per-tool `approval_mode = "approve"` (Jon's config already has nine). | openai/codex `core/src/mcp_tool_call.rs` (`requires_mcp_tool_approval`), `config/src/mcp_types.rs` |
| Claude Desktop | Undocumented. Per-tool "always allow" exists in its UI; whether annotations or the `.mcpb` manifest change it is unknown. Task 7 finds out by hand. | — |

Today no tool declares any annotation (`grep annotations crates/ve-app/src/mcp`
is empty), which is why Codex prompts for `layers_list`.

## Global Constraints

- Invariant 5: nothing here opens a socket or fetches. Writing a client's
  configuration happens **only on the person's click** of an Add button, never
  as a side effect of another setting (the rule `mcp::clients` already states).
- `~/.claude.json` is never written by us (it is Claude Code's live state;
  `clients.rs` explains). `~/.claude/settings.json` is a documented,
  hand-edited file and may be edited, keeping every key we do not own.
- `$CLAUDE_CONFIG_DIR` and `$CODEX_HOME` are honoured, as they already are
  (`skill::claude_home`, `clients::codex_home`).
- The default choice is **"Ask only before files, downloads and discarding"**.
  `invoke` always asks in that choice: it can run any command.
- Every new string ships in all nine languages; every new control gets a
  `data-feature` and a Help search entry (CLAUDE.md, *Adding interface text*).
- `cargo fmt`, `clippy -D warnings`, `VE_FORCE_CPU=1 cargo test --workspace`,
  `npm run ui:typecheck`, `npm run ui:test`, `npm run check:offline` before
  each commit that touches their area.

## Review Focus

1. **`settings.json` that is not plain JSON** (a comment, a trailing comma, a
   hand-edit gone wrong): refuse with the file named and leave it byte-for-byte
   untouched. Never "repair" it. — Task 3, test
   `an_unparseable_settings_file_is_refused_and_untouched`.
2. **A rule the person wrote themselves** (e.g. their own
   `mcp__vectoreffects__export_grib` in `allow`, or anything in `deny`/`ask`):
   re-registering must not delete it. We remove only the entries *we* wrote
   last time, which we remember in our own settings. — Task 3, test
   `rules_the_person_wrote_survive_a_reregistration`.
3. **Upgrading to a version with a new tool**: an enumerated allow list goes
   stale. The table test forces every tool to be classified; the dialog says
   to press *Add to Claude Code* again after an update (Task 5), and Codex
   needs nothing (annotations travel with the tool list). — Task 1, test
   `every_tool_is_classified_and_nothing_else_is`.
4. **Changing the choice after registering**: nothing changes in the client
   until Add is pressed again. The dialog says so under the choice. — Task 5,
   component test `changing_the_choice_says_to_press_add_again`.
5. **No `~/.claude` folder, or no `settings.json`** (fresh Claude Code
   install): create `settings.json` with only our `permissions.allow`. A
   missing *folder* means Claude Code is not installed — but `claude mcp add`
   already succeeded by then, so create it. — Task 3, test
   `a_missing_settings_file_is_created`.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/ve-app/src/mcp/tools/effects.rs` (new) | `Effect` enum, the per-tool table, `annotations(Effect)`, `apply(&mut ToolRouter)` |
| `crates/ve-app/src/mcp/tools/mod.rs` | call `effects::apply` after summing the routers |
| `crates/ve-app/src/settings.rs` | `McpAsk` choice on `McpSettings`, plus `claude_rules` (what we last wrote) |
| `crates/ve-app/src/mcp/claude_settings.rs` (new) | pure `settings.json` edit + atomic write |
| `crates/ve-app/src/mcp/clients.rs` | pass the choice to Claude Code and Codex registration |
| `ui/src/settings/McpSection.tsx` (+ test) | the choice, the "press Add again" note |
| `ui/src/i18n/locales/*/settings.ts`, `ui/src/help/…` | strings and Help entry |
| `spec.md` §8.8, `CLAUDE.md` *Adding an MCP tool* | the rule for new tools |

---

### Task 1: Classify every tool and publish annotations

**Files:**
- Create: `crates/ve-app/src/mcp/tools/effects.rs`
- Modify: `crates/ve-app/src/mcp/tools/mod.rs:280-294` (and `mod effects;`)
- Test: `crates/ve-app/tests/mcp.rs`

**Interfaces:**
- Produces: `pub enum Effect { Read, Edit, View, Files, Network, Discard, Anything }`,
  `pub fn effect_of(name: &str) -> Option<Effect>`,
  `pub const TABLE: &[(&str, Effect)]`,
  `pub fn annotations(Effect) -> ToolAnnotations`,
  `pub fn apply<S>(&mut ToolRouter<S>)`. (`runs_without_asking` is Task 2's.)

The classification (54 tools). `Edit` = changes the open document and is undoable;
`View` = frontend state only; `Files` = writes or replaces files on disk;
`Network` = downloads; `Discard` = can throw away unsaved work when told to;
`Anything` = `invoke`.

| Effect | Tools |
|---|---|
| Read | `vectoreffects_guide`, `project_status`, `recent_projects`, `tool_catalogue`, `layers_list`, `objects_list`, `object_get`, `objects_in_region`, `object_tracks`, `field_sample`, `macro_list`, `history_list`, `nrt_products`, `history_archives`, `screenshot` |
| Edit | `layer_add`, `layer_set`, `layer_move`, `layer_remove`, `object_create`, `object_set`, `object_move`, `object_duplicate`, `object_remove`, `keyframe_set`, `keyframe_remove`, `keyframe_move`, `interpolation_set`, `motion_add`, `follow_set`, `timeline_set`, `field_capture`, `field_paste`, `macro_insert`, `storm_create`, `undo`, `redo`, `history_jump`, `import_grib`, `import_zarr`, `import_image`, `export_cancel` |
| View | `view_focus`, `step_set`, `selection_set` |
| Files | `project_save`, `export_grib`, `export_zarr` |
| Network | `import_nrt`, `import_history` |
| Discard | `project_new`, `project_open`, `project_close` |
| Anything | `invoke` |

Before writing the table, run `grep -rn "async fn " crates/ve-app/src/mcp/tools/*.rs`
and reconcile: the test in Step 1 is the authority, and a tool missing above
gets classified by what its command does (read the command, not the
description).

Annotations per effect:

| Effect | readOnly | destructive | idempotent | openWorld |
|---|---|---|---|---|
| Read | true | false | true | false |
| Edit | false | false | false | false |
| View | false | false | true | false |
| Files | false | true | false | false |
| Network | false | false | false | true |
| Discard | false | true | false | false |
| Anything | false | true | false | true |

Under Codex's `auto` that makes Read, Edit and View run and the rest ask —
exactly the default choice, with no Codex configuration at all.

- [ ] **Step 1: Write the failing tests** in `crates/ve-app/tests/mcp.rs`,
  beside `every_tool_schema_is_one_a_strict_client_accepts`, using its
  `mock_app`/`serve`/`client` helpers:

```rust
/// Every tool says what it does to the person's work, as the MCP
/// annotations every client can read (Codex prompts on them). A tool the
/// table does not know, or a table entry no tool has, fails here: that is
/// the step `CLAUDE.md`'s MCP recipe adds for every new tool.
#[tokio::test]
async fn every_tool_is_classified_and_nothing_else_is() {
    let root = TempRoot::new("effects");
    let app = mock_app(&root);
    let (port, token) = serve(&app);
    let client = client(port, &token).await;
    let tools = client.list_all_tools().await.expect("tools");
    let listed: std::collections::BTreeSet<&str> =
        tools.iter().map(|t| t.name.as_ref()).collect();
    let table: std::collections::BTreeSet<&str> =
        ve_app::mcp::tools::effects::TABLE.iter().map(|(n, _)| *n).collect();
    assert_eq!(listed, table, "the effect table and the tool list differ");
    for tool in &tools {
        let a = tool.annotations.as_ref().unwrap_or_else(|| panic!("{} has no annotations", tool.name));
        for (hint, value) in [
            ("readOnlyHint", a.read_only_hint),
            ("destructiveHint", a.destructive_hint),
            ("idempotentHint", a.idempotent_hint),
            ("openWorldHint", a.open_world_hint),
        ] {
            assert!(value.is_some(), "{}: {hint} is missing, which a client reads as true", tool.name);
        }
    }
    client.cancel().await.expect("close");
}

/// The hints that decide a prompt, checked against what the tools do —
/// Codex's rule (`requires_mcp_tool_approval`): destructive asks; read-only
/// runs; otherwise it runs only when neither destructive nor open-world.
#[tokio::test]
async fn the_hints_say_what_the_tools_do() {
    let root = TempRoot::new("hints");
    let app = mock_app(&root);
    let (port, token) = serve(&app);
    let client = client(port, &token).await;
    let tools = client.list_all_tools().await.expect("tools");
    let codex_asks = |name: &str| {
        let a = tools.iter().find(|t| t.name == name).unwrap().annotations.clone().unwrap();
        if a.destructive_hint.unwrap() { return true; }
        if a.read_only_hint.unwrap() { return false; }
        a.open_world_hint.unwrap()
    };
    for runs in ["layers_list", "field_sample", "screenshot", "object_create", "storm_create", "undo", "view_focus", "import_grib"] {
        assert!(!codex_asks(runs), "{runs} should run without asking");
    }
    for asks in ["project_save", "export_grib", "export_zarr", "import_nrt", "import_history", "project_open", "project_close", "project_new", "invoke"] {
        assert!(codex_asks(asks), "{asks} should ask");
    }
    client.cancel().await.expect("close");
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `VE_FORCE_CPU=1 cargo test -p ve-app --test mcp every_tool_is_classified the_hints_say -- --nocapture`
Expected: compile error, `effects` does not exist.

- [ ] **Step 3: Write `effects.rs`** (make `mcp::tools` reachable from the
  test as `ve_app::mcp::tools::effects` — check `lib.rs`/`mcp/mod.rs`
  visibility and make `effects` `pub`):

```rust
//! What each MCP tool does to the person's work, published as the MCP tool
//! annotations (spec.md 8.8).
//!
//! One table, so the annotations, the Claude Code permission rules and the
//! settings dialog cannot disagree. Annotations are hints a client may act
//! on: Codex does (it runs read-only tools, and ones neither destructive nor
//! open-world, without asking); Claude Code does not, which is why
//! `claude_settings` writes rules from this same table.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::ToolAnnotations;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Reads; changes nothing.
    Read,
    /// Changes the open document, undoably.
    Edit,
    /// Moves what the interface shows; not the document.
    View,
    /// Writes or replaces files on disk.
    Files,
    /// Downloads from the internet.
    Network,
    /// Can discard unsaved work when told to.
    Discard,
    /// `invoke`: any command at all.
    Anything,
}

pub const TABLE: &[(&str, Effect)] = &[
    ("vectoreffects_guide", Effect::Read),
    // … every row of the table above, in the order of the groups in mod.rs …
    ("invoke", Effect::Anything),
];

pub fn effect_of(name: &str) -> Option<Effect> {
    TABLE.iter().find(|(n, _)| *n == name).map(|&(_, e)| e)
}

pub fn annotations(effect: Effect) -> ToolAnnotations {
    let (read_only, destructive, idempotent, open_world) = match effect {
        Effect::Read => (true, false, true, false),
        Effect::Edit => (false, false, false, false),
        Effect::View => (false, false, true, false),
        Effect::Files | Effect::Discard => (false, true, false, false),
        Effect::Network => (false, false, false, true),
        Effect::Anything => (false, true, false, true),
    };
    ToolAnnotations::from_raw(None, Some(read_only), Some(destructive), Some(idempotent), Some(open_world))
}

/// Sets every routed tool's annotations from the table. A tool the table
/// does not know is left without, which the integration test refuses.
pub fn apply<S>(router: &mut ToolRouter<S>) {
    for (name, route) in router.map.iter_mut() {
        if let Some(effect) = effect_of(name) {
            route.attr.annotations = Some(annotations(effect));
        }
    }
}
```

  Write out every row of `TABLE` — no "…" in the committed file. Check the
  exact import path of `ToolRouter` against `mod.rs`'s existing `use` and
  `ToolAnnotations::from_raw`'s argument order in
  `rmcp-3.4.0/src/model/tool.rs` (title, read_only, destructive, idempotent,
  open_world, per the macro in `rmcp-macros-3.4.0/src/tool.rs:219-245`).

- [ ] **Step 4: Apply it** in `VectorEffects::new` (`mod.rs:280`):

```rust
        let mut tool_router = Self::tool_router_guide()
            + Self::tool_router_project()
            // … unchanged …
            + Self::tool_router_escape();
        effects::apply(&mut tool_router);
        Self { app, tool_router }
```

- [ ] **Step 5: Run the tests to see them pass**, then the whole `mcp` test
  file: `VE_FORCE_CPU=1 cargo test -p ve-app --test mcp`. Expected: PASS.

- [ ] **Step 6: Commit** — `git add crates/ve-app/src/mcp/tools crates/ve-app/tests/mcp.rs`,
  message "MCP tools declare their effect as annotations; Codex stops asking
  for reads and undoable edits".

---

### Task 2: The choice, as a setting

**Files:**
- Modify: `crates/ve-app/src/settings.rs:486-503`
- Modify: `crates/ve-app/src/mcp/tools/effects.rs` (add `runs_without_asking`)
- Regenerate: `ui/src/generated/*` via `npm run bindings`

**Interfaces:**
- Produces: `pub enum McpAsk { Outside, Everything, Nothing }` (serde
  `snake_case`, ts-rs export `McpAsk.ts`, `Default = Outside`);
  `McpSettings { …, ask: McpAsk, claude_rules: Vec<String> }`;
  `effects::runs_without_asking(Effect, McpAsk) -> bool`.

`Outside` = "ask only before what leaves VectorEffects or cannot be undone";
`Everything` = "always ask" (each client's own default); `Nothing` = "never
ask".

- [ ] **Step 1: Failing tests** in `settings.rs`'s test module:

```rust
#[test]
fn a_settings_file_from_before_the_choice_reads_as_the_default() {
    let old = r#"{"mcp":{"enabled":true,"port":47391,"token":"t"}}"#;
    let s: AppSettings = serde_json::from_str(old).expect("reads");
    assert_eq!(s.mcp.ask, McpAsk::Outside);
    assert!(s.mcp.claude_rules.is_empty());
}
```

and in `effects.rs`:

```rust
#[test]
fn what_runs_without_asking_under_each_choice() {
    use crate::settings::McpAsk::*;
    for e in [Effect::Read, Effect::Edit, Effect::View] {
        assert!(runs_without_asking(e, Outside));
    }
    for e in [Effect::Files, Effect::Network, Effect::Discard, Effect::Anything] {
        assert!(!runs_without_asking(e, Outside));
    }
    assert!(TABLE.iter().all(|&(_, e)| !runs_without_asking(e, Everything)));
    assert!(TABLE.iter().all(|&(_, e)| runs_without_asking(e, Nothing)));
}
```

- [ ] **Step 2: Run, see them fail** (`cargo test -p ve-app --lib settings effects`).
- [ ] **Step 3: Implement.** In `settings.rs`, add `McpAsk` with
  `#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]`,
  `#[serde(rename_all = "snake_case")]`, `#[default] Outside`, and on
  `McpSettings` the two fields with `#[serde(default)]` (check whether
  `McpSettings` already has container-level `#[serde(default)]`; add the
  field-level attribute regardless) and the `Default` impl updated. In
  `effects.rs`:

```rust
pub fn runs_without_asking(effect: Effect, ask: McpAsk) -> bool {
    match ask {
        McpAsk::Everything => false,
        McpAsk::Nothing => true,
        McpAsk::Outside => matches!(effect, Effect::Read | Effect::Edit | Effect::View),
    }
}
```

- [ ] **Step 4: Run tests; `npm run bindings`; `git diff ui/src/generated`
  shows `McpAsk.ts` and the two new fields.** Expected: PASS.
- [ ] **Step 5: Commit** "The MCP service remembers which tools may run without asking".

---

### Task 3: Claude Code permission rules

**Files:**
- Create: `crates/ve-app/src/mcp/claude_settings.rs`
- Modify: `crates/ve-app/src/mcp/mod.rs` (`pub mod claude_settings;`)

**Interfaces:**
- Consumes: `effects::{TABLE, runs_without_asking}`, `McpAsk`, `clients::SERVER_NAME`.
- Produces:
  `pub fn rules_for(ask: McpAsk) -> Vec<String>` and
  `pub fn with_rules(existing: Option<&str>, previous: &[String], rules: &[String]) -> Result<String>` and
  `pub fn write_rules(claude_home: &Path, previous: &[String], rules: &[String]) -> Result<()>`.

Rules: `Nothing` → `["mcp__vectoreffects__*"]`; `Everything` → `[]`;
`Outside` → `mcp__vectoreffects__<tool>` for each tool that runs without
asking, in table order. Enumerated rather than a wildcard plus `ask`
exceptions, so a tool added later asks until it is classified **and** the
person re-registers — the safe failure.

`with_rules` parses `existing` (or `{}` when `None`) as a JSON object, takes
`permissions.allow` (creating `permissions` and `allow` if absent; refusing if
either is not the right type), removes every string equal to one of
`previous`, appends each of `rules` not already present, and serialises with
`serde_json::to_string_pretty` plus a trailing newline. Everything else —
other keys, their order (`preserve_order` is on in this workspace; assert it
in a test so a feature change is caught), `deny`, `ask`,
`defaultMode` — is kept.

- [ ] **Step 1: Failing tests** in `claude_settings.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::McpAsk;

    #[test]
    fn a_missing_settings_file_is_created() {
        let out = with_rules(None, &[], &rules_for(McpAsk::Outside)).expect("edit");
        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        let allow = v["permissions"]["allow"].as_array().expect("allow");
        assert!(allow.iter().any(|r| r == "mcp__vectoreffects__layers_list"));
        assert!(!allow.iter().any(|r| r == "mcp__vectoreffects__invoke"));
        assert!(!allow.iter().any(|r| r == "mcp__vectoreffects__export_grib"));
    }

    #[test]
    fn everything_else_in_the_file_is_kept_in_order() {
        let before = "{\n  \"model\": \"opus\",\n  \"permissions\": {\n    \"deny\": [\"Bash(rm *)\"],\n    \"allow\": [\"Bash(git status)\"]\n  },\n  \"theme\": \"dark\"\n}\n";
        let out = with_rules(Some(before), &[], &rules_for(McpAsk::Nothing)).expect("edit");
        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["model"], "opus");
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["permissions"]["deny"][0], "Bash(rm *)");
        assert_eq!(v["permissions"]["allow"], serde_json::json!(["Bash(git status)", "mcp__vectoreffects__*"]));
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["model", "permissions", "theme"], "preserve_order must be on");
    }

    #[test]
    fn rules_the_person_wrote_survive_a_reregistration() {
        let ours = rules_for(McpAsk::Nothing);
        let before = r#"{"permissions":{"allow":["mcp__vectoreffects__export_grib","mcp__vectoreffects__*"]}}"#;
        // Last time we wrote the wildcard; the person added export_grib by hand.
        let out = with_rules(Some(before), &ours, &rules_for(McpAsk::Everything)).expect("edit");
        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["permissions"]["allow"], serde_json::json!(["mcp__vectoreffects__export_grib"]));
    }

    #[test]
    fn an_unparseable_settings_file_is_refused_and_untouched() {
        let dir = tempfile_dir(); // a TempRoot-style helper local to this module
        let file = dir.join("settings.json");
        let text = "{ \"model\": \"opus\", }\n"; // trailing comma
        std::fs::write(&file, text).unwrap();
        assert!(write_rules(&dir, &[], &rules_for(McpAsk::Outside)).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
    }

    #[test]
    fn a_permissions_value_of_the_wrong_type_is_refused() {
        assert!(with_rules(Some(r#"{"permissions":[]}"#), &[], &rules_for(McpAsk::Outside)).is_err());
        assert!(with_rules(Some(r#"{"permissions":{"allow":"x"}}"#), &[], &rules_for(McpAsk::Outside)).is_err());
    }

    #[test]
    fn every_rule_is_one_claude_code_accepts() {
        // Allow rules need a literal `mcp__<server>__` prefix; the bare
        // server name is not valid there (code.claude.com/docs/en/permissions).
        for ask in [McpAsk::Outside, McpAsk::Nothing] {
            for rule in rules_for(ask) {
                assert!(rule.starts_with("mcp__vectoreffects__"), "{rule}");
            }
        }
    }
}
```

  (`tempfile_dir`: write it as the `TempRoot` pattern used in
  `crates/ve-app/tests/*.rs` — a directory under `std::env::temp_dir()` named
  with the process id and a label, removed on drop. `tempfile` is not a
  dependency; do not add it.)

- [ ] **Step 2: Run, see them fail.**
- [ ] **Step 3: Implement** `rules_for`, `with_rules` and `write_rules`.
  `write_rules` reads `<claude_home>/settings.json` (`NotFound` → `None`;
  `create_dir_all(claude_home)` first), calls `with_rules`, writes a staged
  `.settings.json.vectoreffects` beside it and renames it over the file — the
  pattern of `clients::register_codex`, including the comment about why. Keep
  the existing file's permissions bits (no secret is in it; do not chmod it to
  0600). Errors are `AppError::Doing { doing: "add the MCP service to", what:
  "Claude Code's settings.json", why }` naming the parse error.
- [ ] **Step 4: Run tests, expected PASS.**
- [ ] **Step 5: Commit** "Claude Code's settings.json gains the service's allow rules, and keeps everything else".

---

### Task 4: Registration applies the choice

**Files:**
- Modify: `crates/ve-app/src/mcp/clients.rs:193-223` (`codex_config_with_entry`), `:273-330` (`mcp_register_client`)
- Test: `clients.rs` test module

**Interfaces:**
- Consumes: `McpSettings.ask`, `McpSettings.claude_rules`, `claude_settings::{rules_for, write_rules}`.
- Produces: `codex_config_with_entry(existing, url, token, ask: McpAsk)`.

Codex: `Outside` → remove `default_tools_approval_mode` (Codex's `auto` reads
the annotations of Task 1 and does exactly this); `Everything` → `"prompt"`;
`Nothing` → `"approve"`. Never touch `[mcp_servers.vectoreffects.tools.*]`:
those are the person's own "always allow" answers.

Claude Code: after `register_claude_code` and the skill, `write_rules(&claude_home,
&mcp.claude_rules, &rules_for(mcp.ask))`, then store the new rules in
`session.settings.mcp.claude_rules` and save settings (find how settings are
persisted — `grep -n "fn save" crates/ve-app/src/settings.rs` — and use that
path, inside `with_session`).

- [ ] **Step 1: Failing tests** in `clients.rs` tests:

```rust
#[test]
fn codex_gets_the_approval_mode_annotations_cannot_express() {
    let outside = codex_config_with_entry("", URL, "t", McpAsk::Outside).unwrap();
    assert!(!outside.contains("default_tools_approval_mode"));
    let every = codex_config_with_entry("", URL, "t", McpAsk::Everything).unwrap();
    assert!(every.contains("default_tools_approval_mode = \"prompt\""));
    let never = codex_config_with_entry(&every, URL, "t", McpAsk::Nothing).unwrap();
    assert!(never.contains("default_tools_approval_mode = \"approve\""));
    assert!(!never.contains("\"prompt\""));
}

#[test]
fn the_persons_own_codex_tool_approvals_are_kept() {
    let before = "[mcp_servers.vectoreffects]\nurl = \"x\"\n\n[mcp_servers.vectoreffects.tools.layers_list]\napproval_mode = \"approve\"\n";
    let after = codex_config_with_entry(before, URL, "t", McpAsk::Everything).unwrap();
    assert!(after.contains("[mcp_servers.vectoreffects.tools.layers_list]\napproval_mode = \"approve\""));
}
```

  Update the existing callers in the test module to pass `McpAsk::Outside`.
- [ ] **Step 2: Run, see them fail.**
- [ ] **Step 3: Implement** both changes. For the Codex reference check, run
  `codex mcp get vectoreffects` on a scratch `CODEX_HOME` holding the output,
  as the existing comment at `clients.rs:332` did, and update that comment
  with the version checked.
- [ ] **Step 4: Run** `cargo test -p ve-app --lib clients` and
  `VE_FORCE_CPU=1 cargo test -p ve-app --test mcp`. Expected: PASS.
- [ ] **Step 5: Commit** "Add to Claude Code and Add to Codex apply the choice of what may run without asking".

---

### Task 5: The choice in Settings → MCP

**Files:**
- Modify: `ui/src/settings/McpSection.tsx`, `ui/src/settings/McpSection.test.tsx`
- Modify: `ui/src/i18n/locales/<lang>/settings.ts` ×9 (find the area McpSection's strings use)
- Modify: `ui/src/help/features/<area>.ts`, `ui/src/help/topics.ts` and `ui/src/help/locales/<lang>.ts` ×9 if the MCP help page lists the Add buttons

A three-way choice above the Add buttons, saved to `settings.mcp.ask` the way
the section saves its other fields:

- "Let agents work in VectorEffects without asking; ask before saving, exporting, downloading or discarding" (default)
- "Ask before every action"
- "Never ask"

Under it, always shown: "Applies when you press an Add button. Press it again
after changing this or after updating VectorEffects." The radio group gets
`data-feature="settings:mcp-ask"` and a Help registry entry (label "Agent
permissions", synonyms: permission, prompt, approval, allow, ask, trust).

- [ ] **Step 1: Failing component test** in `McpSection.test.tsx` (follow the
  file's existing happy-dom pattern): rendering with `ask: "outside"` checks
  the first option; choosing "never" calls the settings writer with
  `ask: "nothing"`; the "Press it again" note is in the document — test name
  `changing_the_choice_says_to_press_add_again`.
- [ ] **Step 2: Run** `npm run ui:test -- McpSection`, see it fail.
- [ ] **Step 3: Implement**, strings through `useT()`; add every key to all
  nine locales in the terms of `GLOSSARY.md`/`GLOSSARY.<lang>.md`.
- [ ] **Step 4: Run** `npm run ui:typecheck && npm run ui:test`. Expected:
  PASS, including `coverage.test.ts` and `features.test.ts`.
- [ ] **Step 5: Commit** "Settings → MCP: choose what agents may do without asking".

---

### Task 6: Spec and the recipe

**Files:** `spec.md` §8.8, `CLAUDE.md` (*Adding an MCP tool*), `crates/ve-app/src/mcp/clients.rs` module doc.

- [ ] **Step 1:** In spec §8.8, after the paragraph on *Add to Claude Code* /
  *Add to Codex*, add: the effect table (by group, not tool by tool), the
  three choices, what each Add button writes for each choice (Claude Code:
  `permissions.allow` in `~/.claude/settings.json`, only entries it wrote
  itself replaced; Codex: annotations alone for the default,
  `default_tools_approval_mode` otherwise), and that `invoke` asks unless the
  choice is "Never ask".
- [ ] **Step 2:** In CLAUDE.md's *Adding an MCP tool*, add a step after 1:
  "**Classify it** in `mcp/tools/effects.rs`'s `TABLE` by what its command
  does — `Read`, `Edit` (undoable document change), `View`, `Files`,
  `Network`, `Discard`, `Anything`. That sets its MCP annotations and whether
  Claude Code may run it without asking; `every_tool_is_classified_and_nothing_else_is`
  fails until it is." Stage only that hunk if `CLAUDE.md` has unrelated
  uncommitted edits.
- [ ] **Step 3: Commit** "spec and recipe: every MCP tool declares its effect".

---

### Task 7: Check it in the clients, by hand

No code. Each finding goes into spec §8.8 in the same commit as any fix.
Use `VE_AUTOMATION_ROOT` for any driver run (memory: driver isolation).

- [ ] **Claude Code:** in the running app, Settings → MCP → default choice →
  *Add to Claude Code*. `cat ~/.claude/settings.json` shows the 45 rules and
  nothing else changed (diff against a copy taken first). In a fresh `claude`
  session in an empty folder, ask: "List the VectorEffects layers, then draw
  a 20-knot westerly jet across the North Atlantic." Expected: no prompt.
  Then "Export it as a GRIB to ~/Desktop/test.grib2." Expected: one prompt.
  Switch to *Never ask*, press Add again: the 45 rules are replaced by the
  one wildcard, and the export does not prompt.
- [ ] **Codex:** same two requests in `codex` after *Add to Codex* with the
  default choice. Expected: no prompt, then one prompt. Note that Jon's
  existing nine per-tool approvals are still in `config.toml`.
- [ ] **Claude Desktop:** reinstall the extension (*Add to Claude Desktop*),
  ask the same two things, and record what it does: whether read-only tools
  are grouped or pre-allowed, whether the per-tool settings show the hints.
  If Desktop offers nothing a manifest or annotation can set, say so in spec
  §8.8 and stop — there is no supported lever to pull.
- [ ] **The agent scenarios:** `tools/mcp-scenarios/run.sh` with the three
  usual prompts (memory: MCP agent scenarios), once with
  `VE_SCENARIO_TOOLS=all`, and confirm the transcripts show no permission
  denials for Read/Edit tools.
- [ ] **Commit** the spec notes: "MCP permissions checked in Claude Code, Codex and Claude Desktop".

---

## Considered and not done

- **A skill `allowed-tools` grant.** It lasts only the turn that invokes the
  skill, and would bypass the person's choice of *Ask before every action*
  for that turn. The settings rule is persistent and follows the choice.
- **`mcp__vectoreffects__*` plus `ask` rules for the risky tools** instead of
  an enumerated allow list. Shorter, but a tool added in a later version
  would be allowed before anyone classified it.
- **Writing `.claude/settings.local.json` in a project.** The server is
  registered at user scope so it works in any folder; the rules must be too.
- **Per-tool `approval_mode = "approve"` entries in Codex.** The annotations
  make them unnecessary for the default, and the server-wide key covers
  *Never ask*.
