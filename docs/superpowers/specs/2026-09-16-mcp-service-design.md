# MCP service design

**Date:** 2026-09-16. **Status:** approved in discussion, awaiting spec review.
**Companion to** `spec.md` (invariant 5, §8.6) and `plan.md` §5.

## 1. What this is

An MCP (Model Context Protocol) server built into the application, so an AI
client can open, edit, animate, import, export and look at a project through
the same code paths the interface uses. It is off unless the person switches
it on in Settings, and while it is on the application says so.

Three decisions were taken in discussion and are fixed here:

1. **Transport:** a loopback HTTP listener inside the shipped application,
   gated by the setting. This amends invariant 5 (§7).
2. **Surface:** curated task-level tools plus one raw `invoke` escape hatch.
3. **Consent:** a connected client may do everything the interface can, with
   a visible indicator, and no extra confirmation dialogs.

## 2. Transport and security

- **Server:** `rmcp` (the official Rust SDK) serving Streamable HTTP through
  its tower service, on `hyper`, which the tree already carries. Pure Rust;
  `cargo tree` must show no new `-sys` crate.
- **Bind:** `127.0.0.1` only, IPv4, on the port in settings (default
  `47391`). Binding failure (port taken) is reported in the settings dialog
  and the setting stays on, so the next launch retries.
- **Token:** 32 random bytes, base64url, generated when the setting is turned
  on or the *Rotate token* button is pressed, stored in the settings file
  beside the port. Every request must carry `Authorization: Bearer <token>`;
  anything else gets `401` before any MCP handling. Turning the setting off
  drops the listener and clears the token from the file.
- **Rebinding guard:** `rmcp`'s `allowed_hosts` is `localhost` and
  `127.0.0.1` (with the port), and `allowed_origins` is those two only, so a
  page in a browser cannot reach the server through a rebound name.
- **Lifecycle:** the listener is started by `settings::mcp_set` when the
  setting turns on, and at launch when the saved setting is on. It runs on
  Tauri's tokio runtime; the tokio `JoinHandle` and a `CancellationToken`
  live in `AppState.mcp`. Nothing here runs when the setting is off: the
  module is compiled in, but it creates no socket, thread or task.
- **Module:** `crates/ve-app/src/mcp/` (`server.rs` lifecycle and auth,
  `tools.rs` the tool set, `events.rs` the frontend notifications). Nothing
  outside `settings::mcp_set` and `lib.rs`'s setup calls into it.

## 3. Tool surface

Every tool calls the `#[tauri::command]` function with
`app.state::<AppState>()`, the same `State` Tauri passes. **No tool
duplicates command logic**, and no split is made.

Tools that change the document return the new `ProjectSummary`, so a client
sees `revision`, `dirty` and `can_undo` exactly as the panels do. Errors are
the `AppError` text the interface shows. Directions cross this boundary as
azimuth toward and speeds in m/s, the domain's own units; a client that wants
the project's display convention reads it from the summary.

Names are `snake_case` nouns first. Each parameter struct derives
`schemars::JsonSchema` so the description reaches the client.

| Group | Tools |
|---|---|
| Project | `project_status`, `project_new`, `project_open` (`discard_unsaved`), `project_save` (`path?`, `overwrite`), `project_close` (`discard_unsaved`), `recent_projects` |
| Structure | `layers_list`, `layer_add`, `layer_set` (name, visible, locked, parameter, speed range), `layer_move`, `layer_remove`, `objects_list` (layer?, step?), `object_get`, `object_create` (tool kind, anchor, options as a property map), `object_set` (one or many properties, step, `auto_key`), `object_move`, `object_duplicate`, `object_remove`, `objects_in_region` |
| Time | `keyframe_set`, `keyframe_remove`, `keyframe_move`, `interpolation_set`, `motion_add`, `follow_set`, `timeline_set` (step count, step hours, start time) |
| Field | `field_sample` (points × steps → speed, direction, u, v), `field_capture` (region → clipboard), `field_paste`, `macro_list`, `macro_insert` |
| Files | `import_grib`, `import_image`, `import_history`, `export_grib`, `export_zarr`, `export_cancel` |
| View | `screenshot`, `view_focus` (lon, lat, zoom?), `step_set`, `selection_set` |
| History | `undo`, `redo`, `history_list`, `history_jump` |
| Escape hatch | `invoke` (command name, JSON arguments) |

**`invoke`** dispatches by name to the same implementation functions,
through a table generated beside `generate_handler!`, so a command added
later is reachable the day it lands. Gesture commands (`begin_transform`,
`drag_transform`, `end_gesture`) are reachable this way and nowhere else.

**Long operations** (imports, exports, history) report through MCP progress
notifications on the request's progress token, fed by the same
`export://progress` and `history://progress` payloads the interface reads.

**`object_create`** takes the tool kind and a property map validated by the
tool's `PropertyMap` schema, the same path `create_object` uses, so a client
gets the same defaults, ranges and frozen-option rules the inspector gets.
Sizes are km unless `stamp_space` says otherwise, per spec §3.5.

## 4. The view follows

Driving the backend alone leaves the interface behind (M76). Three events
carry the service's effects to the frontend:

- **`document://changed`** with the `ProjectSummary`, emitted by the service
  after every tool that wrote to the document or opened, saved or closed a
  project. `App.tsx` listens and calls `setProject`. A `null` payload means
  closed: the start screen shows. Because every panel already refreshes by
  `project.revision`, nothing else changes for the document.
- **`view://focus`** with `{lon, lat, zoom?}` and **`view://step`** with the
  step, applied by `MapView` and the timeline. **`view://selection`** with
  object ids, applied through the selection lifecycle so the panels follow.
- The events are emitted **only by the service**. Commands invoked by the
  interface keep returning summaries as they do now; emitting from there too
  would refresh every panel twice per edit.

**Screenshot** is the one tool that needs the frontend's help: the map's
pixels live in the WebGL canvas. The service emits `view://capture` with a
request id; `MapView` reads its framebuffer once the tiles have settled (the
`__veCapture` logic moved into a plain effect; the window global itself
stays dev-only) and returns
the PNG through a new `deliver_capture(id, bytes)` command; the tool waits
35 s: the map's own 10 s for tiles and 20 s for a frame, and a margin.
Nothing is written to disk.

## 5. Settings and indicator

- `AppSettings` gains `mcp: McpSettings { enabled: bool, port: u16, token:
  String }` with `#[serde(default)]`, so older settings files load with it
  off. The token is a plain string in the file the person already owns; it
  grants nothing beyond what sitting at the keyboard grants.
- Commands: `mcp_status` (enabled, port, bound, token, connected clients,
  last tool and time) and `mcp_set(enabled, port)`, `mcp_rotate_token`.
- **Settings dialog,** a new *MCP service* section after *Macros*: the
  toggle; the port field (`NumberField`, `commitWhileTyping={false}`); the
  URL; a copy button for the Claude Code command and one for the Claude
  Desktop config JSON, both with the token filled in; *Rotate token*; and the
  bind error if there is one.
- **Indicator:** while a client session is open the status bar shows
  `MCP: <last tool>` through `hint.ts`'s existing store, and nothing when
  none is. `mcp_status` is polled by the settings dialog only; the badge is
  pushed by a `mcp://activity` event so it costs nothing when idle.

## 6. Spec and rules

- **Invariant 5** gains its second recorded exception, in the shape of the
  M38 one: an *inbound* loopback endpoint that exists only while the person
  has switched it on, answers only to a token they hold, and lives in one
  module nothing else calls. The WebDriver rule stays as it is: that endpoint
  is unauthenticated, drives the interface rather than the domain, and has
  no setting, which is why it is never compiled in. The difference is the
  same one M38 drew: *who asked*.
- `spec.md` §8.6 gets the settings; a new §8.8 describes the service and its
  tool surface; §15 gets a line; `plan.md` §5 gets the reasoning above.
- `CLAUDE.md` invariant 5 gets the matching paragraph, and a recipe *Adding
  an MCP tool*: the call into the `#[tauri::command]` function with
  `app.state::<AppState>()`, the tool in `tools.rs`, the
  `document://changed` emit if it writes, and the integration test.
- `docs/USER-GUIDE.md` and the Help topic for Settings describe the section.

## 7. Testing

- **Rust integration (`crates/ve-app/tests/mcp.rs`):** builds a real
  `AppState` in a temp directory, starts the server on port 0, and drives
  every curated tool with an `rmcp` client over HTTP. Each test asserts the
  document through the returned summary and, where the tool has an IPC
  twin, through the twin's own read (`document_tree`, `object_properties`),
  so a tool that diverged from its command fails.
- **Refusals:** no token → 401; wrong `Host` or `Origin` → refused;
  `project_open` on a dirty project without `discard_unsaved` → the same
  `AppError` the interface gets.
- **Off means off:** with the setting off, `AppState.mcp` holds no handle
  and a connection attempt to the port is refused. With it on and then off,
  the port is released.
- **Frontend (`vitest`, `happy-dom`):** the settings section renders the
  URL and copies the snippets; the `document://changed` handler replaces the
  project and shows the start screen on `null`.
- **End to end:** the existing `ve-driver` opens the dev app with the setting
  on, an MCP client moves an object, and a screenshot shows it moved. This
  is the M76 check for this feature.
- `npm run check:offline` is unchanged in what it reads; the *off means off*
  Rust test is the enforcement for the listener, and a comment beside the
  offline check says so.

## 8. Out of scope

- Remote access, TLS, or any bind other than loopback.
- Per-tool permissions or a read-only mode.
- MCP resources and prompts. Tools only, for now; a `project_status` call
  is cheaper than a resource subscription and the surface stays one thing.
- Exposing the render cache, tiles or evaluator internals.

## 9. Deviations from this design

- The `*_impl` split was not made: every tool calls the `#[tauri::command]`
  function with `app.state::<AppState>()`, as §3 now says.
- A command's `AppError` reaches the client as a tool result with
  `is_error: true` and the error text as content (`ToolError::Refused`), not
  as a JSON-RPC protocol error; only a panic or a malformed request is a
  protocol error (`ToolError::Internal`).
- The whole `mcp` module is generic over `R: tauri::Runtime`, because the
  integration tests drive a `tauri::App<MockRuntime>`; `export_grib`,
  `export_zarr` and `import_history` gained the same generic. Monomorphises
  to Wry in the app.
- `invoke`'s handler table takes `tauri::State<'_, AppState>` rather than the
  app handle, since a `const` table of fn pointers cannot be generic over the
  runtime.
- `field_sample` refuses more than 4096 point×step samples and flattens the
  scene once per step through `commands::sample_points_at_step`, which the
  interface's own `sample_field` also uses.
- Multi-field tools (`layer_set`, `object_set`) apply per command, one undo
  step per field; `write` emits `document://changed` after the closure
  whether it returned Ok or Err, so a partial write is always reported.
  `opened` on that event is true only when the closure actually succeeded —
  a refused `project_open`/`project_new`/`project_close` must not tell the
  frontend a different project opened, which would reset step, selection
  and the active layer out from under an unchanged document.
- The screenshot's pending capture and the progress relay are guards that
  clean up when a cancelled tool future is dropped.
- §3's sentence on units was wrong: the domain sends azimuth toward and
  m/s, the way `spec.md` §8.8 says, not the project's display convention.
  §3 above has been corrected to match; this bullet records the deviation
  from what this design originally specified.
- `tool_catalogue` and `object_tracks` are read-only tools beyond §3's
  table, listed alongside the rest in `spec.md` §8.8.
- The settings file that holds the token is written owner-only (0600) on
  Unix after every save, so the accepted plain-text risk matches its
  justification. Windows has no mode bits and is unchanged.
- `invoke` refuses an argument the command does not declare
  (`deny_unknown_fields`), so a misspelled key is an error the client reads,
  not a default that applies silently.
- MCP tools do not pass through the frontend's `invoke_handler` beta gate
  (`lib.rs`): a process already running when the beta expiry passes keeps
  serving MCP clients after that instant, even while its own UI starts
  refusing every IPC call. The other direction is closed: `setup` returns
  before it reaches the MCP service's start, so no process launched after
  expiry can open the socket at all.
