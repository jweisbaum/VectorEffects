# MCP Service Follow-ups Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the five after-merge items the MCP service's final whole-branch
review listed, each proved by a test, without changing what any tool does for
a client except where the item says so.

**Architecture:** Four of the five are local: a permission bit on the settings
file, a serde attribute in `invoke`'s argument structs, an attribute on the
capture delivery command, and a refusal path for a screenshot the map cannot
take. The fifth is pure motion: `crates/ve-app/src/mcp/tools.rs` (1,596 lines)
becomes a `tools/` directory with one file per tool group, each group an
`impl` block carrying its own `#[tool_router(router = …)]`, and `new` composing
the routers with `+`. The split goes last so the four functional diffs are
reviewed against the file as it is.

**Tech Stack:** Rust (Tauri 2, rmcp 3.4, schemars, serde), TypeScript (React,
Vitest). Tests in `crates/ve-app/tests/mcp.rs` drive the service over real
HTTP against `tauri::test::mock_app()`.

**Spec:** `docs/superpowers/specs/2026-09-16-mcp-service-design.md` (binding),
`spec.md` §8.8, and the final review's findings as recorded in that spec's §9
Deviations. The follow-ups are the review's Minor items 7, 8, 10, 12 and the
`deny_unknown_fields` triage line.

## Global Constraints

- Every tool calls the `#[tauri::command]` function the interface calls, with
  `app.state()`; no command logic is duplicated in the MCP layer (spec §3).
- A command's `AppError` reaches the client as an `is_error` tool result
  (`ToolError::Refused(String)`); only a panic or a malformed request is a
  protocol error (`ToolError::Internal`).
- The `mcp` module is generic over `R: tauri::Runtime`; the tests drive a
  `MockRuntime` app. `invoke::Handler` is the one non-generic shape.
- Every registered command is in `invoke::TABLE` or `invoke::EXCLUDED`; the
  unit test `the_table_covers_every_registered_command` enforces it.
- A command that can take longer than a frame is `#[tauri::command(async)]`.
- No `unwrap()`/`expect()` in non-test code outside genuine startup
  invariants. No `HashMap` iteration in evaluation or export paths.
- Invariant 5: nothing new reaches the network; `npm run check:offline` stays
  green. No new `-sys` crate.
- Frontend types are generated from Rust (`npm run bindings`), never
  hand-written. Long-running commands are labelled in `LONG_RUNNING` in
  `ui/src/ipc.ts` and nowhere else.
- Prose in `spec.md`, `CLAUDE.md` and the design spec wraps near 80 columns.
- Worktree: `.claude/worktrees/mcp-followups`, branch `mcp-followups`, base
  main at 79f9881 (`.gitignore` gained `.claude/worktrees/` in 1f16765).
- Commit messages end with
  `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

---

### Task 1: The settings file is readable by its owner only

**Files:**
- Modify: `crates/ve-app/src/session.rs:273-287` (`save_settings`)
- Test: `crates/ve-app/src/session.rs` (`#[cfg(test)] mod tests`, beside the
  existing `"{ not json"` test near line 437)

**Interfaces:**
- Consumes: `Session::save_settings(&self, settings_file: &Path) -> Result<()>`.
- Produces: nothing new; the file mode changes.

The settings file holds the MCP bearer token in plain text. The design spec
accepted that because the file is the person's own; this makes the mode match
the argument on Unix. Windows has no mode bits, so the change is `cfg(unix)`.

- [ ] **Step 1: Write the failing test**

Append inside `session.rs`'s `mod tests`:

```rust
    #[cfg(unix)]
    #[test]
    fn the_settings_file_is_private_to_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("settings.json");
        // A pre-existing world-readable file must be tightened too, not only
        // a fresh one: `fs::write` keeps an existing file's mode.
        std::fs::write(&file, "{}").expect("seed");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        let session = Session::default();
        session.save_settings(&file).expect("save");
        let mode = std::fs::metadata(&file).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "settings file mode was {mode:o}");
    }
```

Check how the existing tests in that module build a `Session` (they may use a
helper rather than `Default`) and match it. `tempfile` is already a dev
dependency of `ve-app` if any test there uses `tempfile::tempdir()`; if not,
add `tempfile = "3"` under `[dev-dependencies]` in `crates/ve-app/Cargo.toml`.

- [ ] **Step 2: Run it to see it fail**

Run: `VE_FORCE_CPU=1 cargo test -p ve-app --lib session::tests::the_settings_file_is_private_to_its_owner`
Expected: FAIL with `settings file mode was 644`.

- [ ] **Step 3: Set the mode after the write**

In `save_settings`, after the `std::fs::write(...)` line and before `Ok(())`:

```rust
        // The file holds the MCP bearer token in plain text (design spec
        // §7): owner-only is what makes "the file the person already owns"
        // true in practice. `fs::write` keeps an existing file's mode, so
        // this runs after the write, every time.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(settings_file, std::fs::Permissions::from_mode(0o600))
                .doing("restrict the settings file at", settings_file.display())?;
        }
```

`doing` is the `Context` extension already used two lines above; keep the
same import.

- [ ] **Step 4: Run the test and the module**

Run: `VE_FORCE_CPU=1 cargo test -p ve-app --lib session::`
Expected: all pass, the new one included.

- [ ] **Step 5: Record it and commit**

Add to the design spec's §9 Deviations list
(`docs/superpowers/specs/2026-09-16-mcp-service-design.md`):

```
- The settings file that holds the token is written owner-only (0600) on
  Unix after every save, so the accepted plain-text risk matches its
  justification. Windows has no mode bits and is unchanged.
```

```bash
cargo fmt --all
git add crates/ve-app/src/session.rs docs/superpowers/specs/2026-09-16-mcp-service-design.md
git commit -m "Write the settings file owner-only, since it holds the MCP token

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: `invoke` refuses an argument it does not know

**Files:**
- Modify: `crates/ve-app/src/mcp/invoke.rs:36-48` (the `command!` macro)
- Test: `crates/ve-app/tests/mcp.rs` (beside
  `invoke_reaches_a_command_no_curated_tool_covers`)

**Interfaces:**
- Consumes: the `command!` macro's generated `Args` struct.
- Produces: a refusal naming the unknown field, e.g.
  `rename_project: unknown field `nmae`, expected `name``.

A misspelled key in `invoke`'s `args` is silently dropped today: an optional
field falls back to its default and a required one produces an unrelated
"missing field" error. For a model client, the loud refusal is the useful
behaviour.

- [ ] **Step 1: Write the failing test**

Append to `crates/ve-app/tests/mcp.rs`:

```rust
#[tokio::test]
async fn invoke_refuses_an_argument_the_command_does_not_have() {
    let root = TempRoot::new("invoke-unknown");
    let app = mock_app(&root);
    let (port, token) = serve(&app);
    let client = client(port, &token).await;
    call(&client, "project_new", new_project_args("Unknown")).await;
    let message = call_err(
        &client,
        "invoke",
        json!({ "command": "rename_project", "args": { "nmae": "Typo" } }),
    )
    .await;
    assert!(message.contains("unknown field"), "{message}");
    assert!(message.contains("nmae"), "{message}");
    // Nothing was written: the project keeps its name.
    let current = ve_app::projects::current(app.state::<AppState>().inner())
        .expect("current")
        .expect("open");
    assert_eq!(current.name, "Unknown");
    client.cancel().await.expect("close");
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `VE_FORCE_CPU=1 cargo test -p ve-app --test mcp invoke_refuses_an_argument`
Expected: FAIL — today the call succeeds or fails with `missing field `name``,
so the first assertion is what fails.

- [ ] **Step 3: Deny unknown fields in the macro**

In `invoke.rs`'s `command!` macro, change the generated struct:

```rust
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args { $($field: $ty),* }
```

and extend the macro's doc comment with one sentence:

```rust
/// Unknown keys are refused rather than ignored: a client that misspells an
/// argument hears about it instead of watching the default apply.
```

- [ ] **Step 4: Run the invoke tests**

Run: `VE_FORCE_CPU=1 cargo test -p ve-app --test mcp invoke` and
`cargo test -p ve-app --lib mcp::invoke`
Expected: all pass (the coverage test is unaffected; the existing reach test
still passes because its arguments are exact).

- [ ] **Step 5: Record it and commit**

Design spec §9 bullet:

```
- `invoke` refuses an argument the command does not declare
  (`deny_unknown_fields`), so a misspelled key is an error the client reads,
  not a default that applies silently.
```

```bash
cargo fmt --all
git add crates/ve-app/src/mcp/invoke.rs crates/ve-app/tests/mcp.rs docs/superpowers/specs/2026-09-16-mcp-service-design.md
git commit -m "Refuse unknown arguments through the MCP invoke hatch

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: `deliver_capture` runs off the main thread

**Files:**
- Modify: `crates/ve-app/src/mcp/capture.rs:127`

**Interfaces:**
- Consumes: `deliver_capture(service: State<McpService>, id: u64, png_base64: String) -> Result<()>`.
- Produces: the same function; only the Tauri attribute changes.

The command decodes a full-window PNG from base64 inline. A plain
`#[tauri::command]` on a sync function runs on the main thread, which is the
webview's (CLAUDE.md, "A command that can take longer than a frame"). The
`async` attribute moves the same sync function to Tauri's pool; the function
itself stays sync, so `capture.rs`'s own test that calls it directly is
unchanged. It is not added to `LONG_RUNNING`: the spinner is for work the
person waits on, and this answers a client.

- [ ] **Step 1: Change the attribute**

```rust
/// The frontend's answer to `view://capture`.
///
/// `async` so the base64 decode of a full-window PNG runs on Tauri's pool
/// rather than the webview's thread; the function stays synchronous and is
/// called directly by the tests.
#[tauri::command(async)]
pub fn deliver_capture(
```

- [ ] **Step 2: Build and run the capture tests**

Run: `cargo test -p ve-app --lib mcp::capture` and
`VE_FORCE_CPU=1 cargo test -p ve-app --test mcp a_screenshot`
Expected: all pass. `cargo clippy -p ve-app --all-targets -- -D warnings` clean.

- [ ] **Step 3: Commit**

```bash
git add crates/ve-app/src/mcp/capture.rs
git commit -m "Decode a delivered capture off the main thread

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: A screenshot the map cannot take fails fast

**Files:**
- Modify: `crates/ve-app/src/mcp/capture.rs` (`Captures`, `PendingCapture`,
  new command `refuse_capture`)
- Modify: `crates/ve-app/src/mcp/tools.rs:1152-1181` (`screenshot`)
- Modify: `crates/ve-app/src/lib.rs:272` (`generate_handler!`)
- Modify: `crates/ve-app/src/mcp/invoke.rs:85` (`EXCLUDED`)
- Modify: `ui/src/ipc.ts:664` (`api.refuseCapture`)
- Modify: `ui/src/map/MapView.tsx:5554-5567` (the `view://capture` listener)
- Modify: `spec.md` §8.8, the design spec §4 and §9
- Test: `crates/ve-app/src/mcp/capture.rs` (unit), `crates/ve-app/tests/mcp.rs`

**Interfaces:**
- Consumes: `Captures::request`, `PendingCapture::receiver`, `deliver_capture`.
- Produces: `pub type CaptureAnswer = std::result::Result<Vec<u8>, String>;`
  `PendingCapture::receiver() -> &mut oneshot::Receiver<CaptureAnswer>`;
  command `refuse_capture(id: u64, reason: String) -> Result<()>`;
  `api.refuseCapture(id: number, reason: string): Promise<void>`.

Two cases cost a client 35 s today: no project is open (the map is not
mounted, so nothing answers), and the map's `framebufferPng()` returns `null`
(no frame within its own race). The first is refused before any request is
made; the second is answered by the frontend with a reason the tool returns
immediately.

- [ ] **Step 1: Write the failing unit test**

In `capture.rs`'s `mod tests`:

```rust
    /// The frontend can decline a request, and the decline reaches the
    /// waiting tool as a reason rather than as silence.
    #[test]
    fn a_refused_request_delivers_its_reason() {
        let app = tauri::test::mock_app();
        let captures = Captures::default();
        let mut pending = captures.request(app.handle());
        captures
            .refuse(pending.id(), "no frame".to_owned())
            .expect("refuse a live request");
        assert_eq!(
            pending.receiver().try_recv().ok(),
            Some(Err("no frame".to_owned()))
        );
        // Refusing again finds nothing: the entry went with the answer.
        assert!(captures.refuse(pending.id(), "again".to_owned()).is_err());
    }
```

Update the existing `a_dropped_request_is_forgotten` test's expectation from
`Some(vec![1, 2, 3])` to `Some(Ok(vec![1, 2, 3]))`.

- [ ] **Step 2: Write the failing integration tests**

Append to `crates/ve-app/tests/mcp.rs`:

```rust
#[tokio::test]
async fn a_screenshot_with_no_project_open_is_refused_at_once() {
    let root = TempRoot::new("shot-none");
    let app = mock_app(&root);
    let (port, token) = serve(&app);
    let client = client(port, &token).await;
    let started = std::time::Instant::now();
    let message = call_err(&client, "screenshot", json!({})).await;
    assert!(message.contains("No project is open"), "{message}");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    client.cancel().await.expect("close");
}

#[tokio::test]
async fn a_screenshot_the_map_declines_fails_with_the_reason() {
    let root = TempRoot::new("shot-declined");
    let app = mock_app(&root);
    let (port, token) = serve(&app);
    let client = client(port, &token).await;
    call(&client, "project_new", new_project_args("Declined")).await;
    // Stand in for MapView with no frame to give.
    let handle = app.handle().clone();
    app.listen(ve_app::mcp::capture::CAPTURE, move |event| {
        let id = serde_json::from_str::<Value>(event.payload()).expect("json")["id"]
            .as_u64()
            .expect("id");
        ve_app::mcp::capture::refuse_capture(handle.state(), id, "no frame to capture".to_owned())
            .expect("refuse");
    });
    let started = std::time::Instant::now();
    let message = call_err(&client, "screenshot", json!({})).await;
    assert!(message.contains("no frame to capture"), "{message}");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    client.cancel().await.expect("close");
}
```

`call_err` takes a `Value` for arguments today; if it takes `Option<Value>`
or the screenshot test in the file passes `arguments: None`, match the file.

- [ ] **Step 3: Run them to see them fail**

Run: `cargo test -p ve-app --lib mcp::capture` (compile error: no `refuse`)
and `VE_FORCE_CPU=1 cargo test -p ve-app --test mcp a_screenshot_`
(compile error: no `refuse_capture`; and the no-project test would wait 35 s
and fail on the elapsed assertion if the others compiled).

- [ ] **Step 4: The answer becomes a `Result`**

In `capture.rs`:

```rust
/// What the frontend answers with: the PNG, or why there is none.
pub type CaptureAnswer = std::result::Result<Vec<u8>, String>;
```

Change `pending`'s type to
`Mutex<HashMap<u64, tokio::sync::oneshot::Sender<CaptureAnswer>>>`, the
channel in `request` to `oneshot::channel::<CaptureAnswer>()`, `PendingCapture::rx`
and `receiver()` to `CaptureAnswer`. Replace `deliver` with a private helper
both answers share:

```rust
    /// Answers a request, removing it. A closed receiver means the tool gave
    /// up first; the answer is dropped, which is not the frontend's failure.
    fn answer(&self, id: u64, answer: CaptureAnswer) -> Result<()> {
        let sender = self.pending.lock().ok().and_then(|mut p| p.remove(&id));
        match sender {
            Some(tx) => {
                let _ = tx.send(answer);
                Ok(())
            }
            None => Err(AppError::BadOption {
                field: "capture id",
                value: id.to_string(),
            }),
        }
    }

    fn deliver(&self, id: u64, png: Vec<u8>) -> Result<()> {
        self.answer(id, Ok(png))
    }

    /// The frontend had no picture to give; the tool hears why at once
    /// instead of waiting out its timeout.
    fn refuse(&self, id: u64, reason: String) -> Result<()> {
        self.answer(id, Err(reason))
    }
```

Add the command beside `deliver_capture`:

```rust
/// The frontend's "no" to `view://capture`: the map had no frame to give
/// (no project, window not shown, or its own capture race lost).
#[tauri::command(async)]
pub fn refuse_capture(
    service: tauri::State<'_, super::McpService>,
    id: u64,
    reason: String,
) -> Result<()> {
    service.captures.refuse(id, reason)
}
```

Register it in `crates/ve-app/src/lib.rs`'s `generate_handler!` after
`mcp::capture::deliver_capture`, and add `"refuse_capture"` to
`invoke::EXCLUDED` next to `"deliver_capture"` (the coverage test fails until
both are done).

- [ ] **Step 5: The tool refuses early and reads the reason**

In `tools.rs`, replace the body of `screenshot`:

```rust
    async fn screenshot(&self) -> std::result::Result<CallToolResult, ToolError> {
        // Neither `run` nor `write`: the work is the frontend's, so there is
        // no closure to put on `spawn_blocking` and no document to report a
        // change to. The activity note is what the other two would have done.
        let service = self.app.state::<super::McpService>();
        service.note_tool(&self.app, "screenshot");
        // With no project the map is not mounted and nothing would answer;
        // refuse now rather than after the timeout.
        if crate::projects::current(&self.app.state::<AppState>())?.is_none() {
            return Err(ToolError::from(AppError::NoProjectOpen));
        }
        // The guard forgets the request on every way out of here, the
        // dropped future included, so nothing has to be cleaned up by hand.
        let mut pending = service.captures.request(&self.app);
        // The frontend waits up to 10 s for tiles and 20 s for a frame (M78);
        // a little longer than both, then give up rather than hang the client.
        let answer =
            tokio::time::timeout(std::time::Duration::from_secs(35), pending.receiver()).await;
        match answer {
            Ok(Ok(Ok(png))) => {
                let data = base64::engine::general_purpose::STANDARD.encode(png);
                Ok(CallToolResult::success(vec![ContentBlock::image(
                    data,
                    "image/png",
                )]))
            }
            Ok(Ok(Err(reason))) => Err(ToolError::Refused(format!(
                "the map could not take the picture: {reason}"
            ))),
            _ => Err(ToolError::Internal(McpError::internal_error(
                "the map did not answer the capture: is the window shown?",
                None,
            ))),
        }
    }
```

`crate::projects::current` takes `&AppState`; `self.app.state::<AppState>()`
is a `State`, which derefs to it — write `&*self.app.state::<AppState>()` if
the compiler asks. The `?` converts the `AppError` through `From`.

- [ ] **Step 6: The frontend says no instead of nothing**

`ui/src/ipc.ts`, after `deliverCapture`:

```ts
  /** The map's "no" to a `view://capture` request, with the reason. */
  refuseCapture: (id: number, reason: string) =>
    call<void>("refuse_capture", { id, reason }),
```

`ui/src/map/MapView.tsx`, the listener body:

```ts
    const pending = listen<CaptureRequest>("view://capture", async (event) => {
      const png = await framebufferPng();
      try {
        if (png === null) {
          // Say so at once: the tool would otherwise wait out its timeout
          // and report silence it cannot explain.
          await api.refuseCapture(
            event.payload.id,
            "no frame to capture: is the window shown and the map settled?",
          );
          return;
        }
        await api.deliverCapture(event.payload.id, png);
      } catch (err) {
        void api.frontendLog("warn", `capture ${event.payload.id} not answered: ${String(err)}`);
      }
    });
```

- [ ] **Step 7: Run everything this touched**

```bash
cargo test -p ve-app --lib mcp::
VE_FORCE_CPU=1 cargo test -p ve-app --test mcp
cargo clippy -p ve-app --all-targets -- -D warnings && cargo fmt --all
npm run bindings && git diff --stat -- ui/src/generated crates/ve-app/bindings
npm run ui:typecheck && npm run ui:test
npm run check:offline
```

Expected: all green; the coverage test passes with `refuse_capture` excluded;
bindings show no drift (no `ts-rs` type changed).

- [ ] **Step 8: Record it in the spec and the design spec**

`spec.md` §8.8, the sentence "The `screenshot` tool asks the map for its own
framebuffer through `view://capture` and `deliver_capture`; nothing is
written to disk." becomes:

```
The `screenshot` tool asks the map for its own framebuffer through
`view://capture`, answered by `deliver_capture` or, when the map has no frame
to give, by `refuse_capture` with the reason; it is refused at once with no
project open. Nothing is written to disk.
```

Design spec §4's screenshot paragraph: add "A frontend that cannot produce a
frame answers `refuse_capture(id, reason)` and the tool returns the reason as
a refusal; with no project open the tool refuses before asking." Design spec
§9: one bullet saying the same in a line.

```bash
git add crates/ve-app/src ui/src spec.md docs/superpowers/specs/2026-09-16-mcp-service-design.md
git commit -m "Fail an MCP screenshot fast when the map has nothing to give

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: `tools.rs` becomes a directory, one file per tool group

**Files:**
- Create: `crates/ve-app/src/mcp/tools/mod.rs`, `project.rs`, `structure.rs`,
  `time.rs`, `field.rs`, `files.rs`, `view.rs`, `history.rs`, `escape.rs`
- Delete: `crates/ve-app/src/mcp/tools.rs`
- Modify: `CLAUDE.md` (the "Adding an MCP tool" recipe names `mcp/tools.rs`),
  `docs/superpowers/specs/2026-09-16-mcp-service-design.md` (every
  `tools.rs` mention), `spec.md` §8.8 if it names the file
- Test: existing `crates/ve-app/tests/mcp.rs` (unchanged) and the unit tests
  that move with their code

**Interfaces:**
- Consumes: everything `tools.rs` defines today.
- Produces: the same public surface under `crate::mcp::tools::*`: `VectorEffects<R>`,
  `VectorEffects::new`, `ToolError`, `Done`, the parameter and result structs
  (re-exported from `mod.rs` with `pub use`), and the `ServerHandler` impl.
  Router composition: `Self::tool_router_project() + Self::tool_router_structure()
  + Self::tool_router_time() + Self::tool_router_field() + Self::tool_router_files()
  + Self::tool_router_view() + Self::tool_router_history() + Self::tool_router_escape()`.

This is motion, not change: no tool's name, description, parameters or
behaviour moves. The integration tests are the proof, because they list and
call the tools over HTTP by name; any tool that fails to land in a router
disappears from `tools/list` and `the_right_token_initialises_and_lists_tools`
or a call fails.

`rmcp-macros` 3.4 supports several routers on one type:
`#[tool_router(router = tool_router_project, vis = "pub(crate)")]` on an
`impl` block generates `pub(crate) fn tool_router_project() -> ToolRouter<Self>`,
and `ToolRouter` implements `Add`, so the routers compose with `+`. The
`#[tool_handler(router = self.tool_router.clone())]` on the `ServerHandler`
impl is unchanged.

Group membership is spec §8.8's, by tool name:

| file | tools |
|---|---|
| `project.rs` | `project_status`, `project_new`, `project_open`, `project_save`, `project_close`, `recent_projects`, `tool_catalogue` |
| `structure.rs` | `layers_list`, `layer_add`, `layer_set`, `layer_move`, `layer_remove`, `objects_list`, `object_get`, `object_create`, `object_set`, `object_move`, `object_duplicate`, `object_remove`, `objects_in_region` |
| `time.rs` | `keyframe_set`, `keyframe_remove`, `keyframe_move`, `interpolation_set`, `motion_add`, `follow_set`, `timeline_set`, `object_tracks` |
| `field.rs` | `field_sample`, `field_capture`, `field_paste`, `macro_list`, `macro_insert` (and `MAX_FIELD_SAMPLES`) |
| `files.rs` | `import_grib`, `import_image`, `import_history`, `export_grib`, `export_zarr`, `export_cancel` (and `ProgressRelay`, `relay_progress`, the relay's unit test) |
| `view.rs` | `screenshot`, `view_focus`, `step_set`, `selection_set` |
| `history.rs` | `undo`, `redo`, `history_list`, `history_jump` |
| `escape.rs` | `invoke` (`InvokeParams`, `Invoked`) |

- [ ] **Step 1: Record the tool list before moving anything**

```bash
grep -c '#\[tool(' crates/ve-app/src/mcp/tools.rs      # expect 48
awk '/#\[tool\(/{armed=1} armed && match($0,/fn [a-z_]+/){print substr($0,RSTART+3,RLENGTH-3); armed=0}' crates/ve-app/src/mcp/tools.rs | sort > /tmp/tools-before.txt
```

- [ ] **Step 2: Create `tools/mod.rs` with the shared machinery**

```bash
mkdir -p crates/ve-app/src/mcp/tools
git mv crates/ve-app/src/mcp/tools.rs crates/ve-app/src/mcp/tools/mod.rs
```

`mod.rs` keeps: the module doc comment, the imports it still needs,
`VectorEffects<R>` and its `Debug`/`Drop` impls, `ToolError` and its impls,
the `impl<R> VectorEffects<R>` block holding `run` and `write`, `Done`, the
`ServerHandler` impl with `#[tool_handler(router = self.tool_router.clone())]`,
and a `new` that composes:

```rust
mod escape;
mod field;
mod files;
mod history;
mod project;
mod structure;
mod time;
mod view;

pub use escape::{InvokeParams, Invoked};
pub use field::*;
pub use files::*;
pub use history::*;
pub use project::*;
pub use structure::*;
pub use time::*;
pub use view::*;

impl<R: tauri::Runtime> VectorEffects<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        // One router per group, each generated by its file's `#[tool_router]`;
        // `ToolRouter` adds. A group left out of this sum is a group of tools
        // no client can list, which the integration tests catch by name.
        let tool_router = Self::tool_router_project()
            + Self::tool_router_structure()
            + Self::tool_router_time()
            + Self::tool_router_field()
            + Self::tool_router_files()
            + Self::tool_router_view()
            + Self::tool_router_history()
            + Self::tool_router_escape();
        Self { app, tool_router }
    }
}
```

If a parameter or result struct is shared by two groups, it stays in `mod.rs`
(as `Done` does) rather than being re-exported twice.

- [ ] **Step 3: Move each group into its file**

For every file in the table: the section's parameter and result structs, then
one block

```rust
#[tool_router(router = tool_router_<group>, vis = "pub(crate)")]
impl<R: tauri::Runtime> VectorEffects<R> {
    // the group's #[tool] fns, moved verbatim
}
```

with `use super::{Done, ToolError, VectorEffects};` and whatever else the
moved code names (`rmcp::handler::server::wrapper::{Json, Parameters}`,
`rmcp::{tool, tool_router}`, `schemars::JsonSchema`, `serde::{Deserialize,
Serialize}`, the `crate::` paths the tools call). Use `cargo check -p ve-app`
after each file; unresolved names are the compiler telling you what to import.
`files.rs` takes `ProgressRelay`, `relay_progress` and the
`a_dropped_relay_stops_listening` test with it; `field.rs` takes
`MAX_FIELD_SAMPLES`.

Do not edit a `#[tool(description = …)]` string, a struct field, a doc
comment or a function body while moving it. If something in a body needs a
`super::` prefix to resolve, that prefix is the only change.

- [ ] **Step 4: Prove nothing moved but the code**

```bash
awk '/#\[tool\(/{armed=1} armed && match($0,/fn [a-z_]+/){print substr($0,RSTART+3,RLENGTH-3); armed=0}' crates/ve-app/src/mcp/tools/*.rs | sort > /tmp/tools-after.txt
diff /tmp/tools-before.txt /tmp/tools-after.txt && echo "same 48 tools"
wc -l crates/ve-app/src/mcp/tools/*.rs
cargo test -p ve-app --lib mcp::
VE_FORCE_CPU=1 cargo test -p ve-app --test mcp
cargo clippy -p ve-app --all-targets -- -D warnings && cargo fmt --all
npm run bindings && git diff --stat -- ui/src/generated crates/ve-app/bindings
```

Expected: the tool lists are identical; every file is under 500 lines; all
tests pass; no bindings drift.

- [ ] **Step 5: Update the documents that name the file**

`CLAUDE.md`, recipe "Adding an MCP tool", step 2: "a `schemars::JsonSchema`
struct in `mcp/tools/<group>.rs` beside the tool, with a doc comment per
field" — and add a step 0 line: "Pick the group file in `mcp/tools/`
(project, structure, time, field, files, view, history); a new group is a new
file with its own `#[tool_router(router = …)]` block added to the sum in
`tools/mod.rs`'s `new`." Renumber. In the design spec, replace `tools.rs`
with `tools/` where it names the file, and add a §9 bullet:

```
- `mcp/tools.rs` is a directory: one file per spec §8.8 group, each an
  `impl` block with its own `#[tool_router(router = …)]`, composed with `+`
  in `tools/mod.rs`. Motion only; no tool changed.
```

Check `spec.md` §8.8 for a `tools.rs` mention (`grep -n "tools.rs" spec.md`)
and update it the same way if there is one.

- [ ] **Step 6: Commit**

```bash
git add -A crates/ve-app/src/mcp/tools crates/ve-app/src/mcp CLAUDE.md spec.md docs/superpowers/specs/2026-09-16-mcp-service-design.md
git commit -m "Split the MCP tools into one file per group

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: Full verification

- [ ] **Step 1: Everything**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
VE_FORCE_CPU=1 cargo test --workspace
npm run bindings && git diff --exit-code -- ui/src/generated crates/ve-app/bindings
npm run ui:typecheck && npm run ui:test
npm run ui:build && npm run check:offline
cargo tree -p ve-app -e normal | grep -oE '[a-z0-9_-]+-sys v[0-9.]+' | sort -u
```

The last line must list only `aws-lc-sys`, `core-foundation-sys`, `dirs-sys`,
`security-framework-sys` and `zstd-sys` on macOS.

- [ ] **Step 2: The screenshot round trip in the real application**

`node tools/webdriver/mcp-follow.mjs` still prints `OK: … -> …` (it takes two
captures through the same `view://capture` path Task 4 changed). This needs a
display session; it is not CI.

- [ ] **Step 3: Finish the branch**

Use `superpowers:finishing-a-development-branch`.
