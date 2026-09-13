**Draft plan: timeline playback at the requested frame rate**

The goal is to display each successive timeline step at the requested average FPS once playback preparation is complete. Preserve the existing behavior of holding on an unavailable frame and resuming at the next step without skipping. Measure the map's displayed steps, not just changes to the timeline playhead.

This plan is based on the current working tree, including its existing uncommitted playback changes. Those changes already introduce simultaneous two-step warming, a viewport-sized texture reservation capped at 1 GiB, and asynchronous render-ahead/readiness commands. They need validation as part of the solution.

**Findings from inspection and a clock simulation**

| Finding | Evidence | Consequence |
| --- | --- | --- |
| The playback clock discards every advance's timing remainder. | `Timeline.tsx` sets `lastAdvance = now`; `playback.ts` requires a full interval before advancing again. A 60-second simulation using the current `tick` function and ideal 60 Hz callbacks, with all frames solid and resident, yields 7.5 FPS at a requested 8 and 20 FPS at a requested 24. | Playback can undershoot the selected FPS even when rendering, fetching, and drawing cost nothing. |
| Background completion covers backend tiles, not complete display preparation. | `render_pool.rs` reports backend cache presence. `tiles.ts` separately resolves keys, fetches bytes, scans speed ranges, and uploads WebGL textures. Timeline lookahead runs while playing. | The first playback can still buffer after the readiness strip becomes solid. |
| Moving the playhead repeats work for the whole timeline. | The step-dependent effect in `Timeline.tsx` calls both `renderAhead` and `frameReadiness`. `RenderPool::request` rebuilds the entire queue, including cached tiles; `readiness` probes every step and viewport tile. | A completed background render still incurs queue processing and readiness work on every advance. |
| Playback preparation repeats some backend scene work. | `protocol.rs` uses a separate three-entry `SceneCache`; `tile_keys` has a synchronous command declaration and computes tile keys separately from the pool's prepared scenes. | Key resolution, scene-cache misses, and session-lock contention are candidates for further delays; their contribution needs profiling. |
| Texture capacity and frame metadata retention are different limits. | `tiles.ts` reserves textures up to 1 GiB but retains only 96 frame key maps. | Long loops can repeatedly resolve keys even when shared textures fit; larger animated loops can also evict and reload textures. |
| Advancing the playhead is separate from drawing the map. | `MapView.tsx` mirrors the step through a React effect and schedules another animation callback. Timeline trees, expanded tracks, and inspector data also refresh by step. | Playhead timing alone cannot establish visible FPS, and editor updates may compete with presentation. |

The existing focused suites pass: 44 tests across `playback.test.ts` and `tiles.test.ts`. They cover individual clock decisions and cache rules, but not sustained FPS or the complete presentation path. No live-app FPS measurement has been made for this draft; the clock defect is reproduced, while the relative costs of the other paths remain to be measured.

1. **Establish an end-to-end baseline.**

   Add opt-in, bounded diagnostics for scheduled advances, map draws with a fully resident frame token, buffering causes, key-resolution latency, tile transfers, range scanning, uploads, cache evictions, backend queue activity, and readiness probes. Keep timings out of per-tile production logging. Distinguish animation callback rate, requested steps, and distinct steps actually drawn; verify visible cadence in the real webview alongside those counters.

   Use the existing WebDriver tooling with an isolated animated fixture and, when available, a copy of the affected project. Measure first playback after backend completion and subsequent loops separately. Record build mode, machine, viewport, device-pixel ratio, display refresh, requested FPS, glyph settings, and panel state. Fix the existing backend cost fixture before relying on it: its fixed zoom and coordinate generation produce invalid addresses for the larger viewport cases, and empty scenes do not represent animated tile traffic.

2. **Correct the clock without skipping frames.**

   Extract a stateful, testable scheduler from the `Timeline.tsx` effect. Use a monotonic deadline and carry the fractional remainder during normal playback: advance the deadline by the interval instead of restarting it at the callback timestamp. Permit at most one sequential step per display opportunity. This lets 24 FPS alternate between display intervals as necessary on a 60 Hz screen.

   On buffering, a suspension, or a delay of multiple intervals, discard accumulated timing debt and rebase when the next frame can be shown. Do not burst through frames to catch up. Reset timing deliberately on play, pause/resume, stop, scrubbing, rate changes, and macro-preview transitions. Test decimal rates and floating-point deadline boundaries as well as integer rates.

3. **Prepare playback before frames are due.**

   Introduce a preparation coordinator in the map/tile-cache path. It should continue bounded preparation while paused as backend tiles become available, with the current frame and the next playback frames first. Feed key resolution directly into queued fetches and uploads rather than requiring another animation callback to move each stage forward. Bound concurrent requests and upload work so preparation cannot monopolize presentation.

   Replace fixed two-step scheduling with a lookahead sized from requested FPS, measured preparation latency, and the memory budget. Retain content-hash sharing. Pin the displayed frame and the protected playback window against eviction; release pins when the viewport, revision, run, or project changes. Keep key maps for the active timeline independently of expensive texture storage, replacing the fixed 96-frame limit with bounded metadata retention that covers supported timeline lengths.

   When the timeline fits, prepare and retain it for immediate playback and repeated loops. When it does not, use a bounded rolling buffer and measure whether sustained preparation can feed the requested rate. Expose a concise preparation/buffering status so backend rendering completion is not mistaken for full display preparation. Do not promise full residency beyond the memory budget or assume that increasing the 1 GiB cap solves throughput.

4. **Stop rebuilding completed background work on every step.**

   Separate changes to document/viewport identity from changes to playhead priority. Rebuild work when the document or viewport changes; moving the playhead should only reprioritize outstanding work. Once the viewport's timeline is complete, leave workers idle until invalidation or an actual cache miss requires work.

   Make readiness updates event-driven and coalesced, with at most one probe in flight and one pending refresh. Reuse unchanged results and avoid React updates for identical reports. Account for backend cache eviction so an old completion report does not become permanent truth. Reject responses from obsolete project, revision, viewport, or request generations before updating either readiness state or its history.

5. **Remove measured preparation and editor costs from presentation.**

   Share prepared whole-scene frames and tile keys between background rendering and tile serving, keyed by revision, step, and scope. Preserve separate identities for macro previews and layer-only/without-layer frames. Move expensive key-resolution commands off the webview thread and minimize time spent holding the session lock. Use bounded caching and single-flight preparation so concurrent requests reuse the same work.

   Give playback an explicit map presentation request and draw acknowledgment. Keep the controller's requested step separate from its last presented step, and prevent a subsequent advance from overwriting a step that has not been drawn. Synchronize the visible playhead with presentation while preserving immediate manual scrubbing.

   Profile React commits and step-dependent IPC with expanded tracks, selections, and inspector panels. Cache document structure by revision, derive simple active-step flags locally, share duplicate queries, and coalesce expensive editor refreshes where appropriate. Restore exact editor values on pause or scrub. Avoid fetching edit-only layer scopes during passive playback if the measurements show that warming them competes with playback. Optimize glyph/compositing work only if it remains a measured bottleneck; a separate rendered-image playback cache would need its own memory and invalidation design.

6. **Validate the result against explicit acceptance criteria.**

   Deterministic scheduler tests should stay within one expected advance over 60 seconds at 8, 12, 24, and 30 FPS under ideal 60/120 Hz callbacks. Add jitter, fractional rates, changing rates, pauses, loop boundaries, one-step runs, delayed presentation, buffering, and suspension. Frame order must remain sequential with no catch-up skips.

   In the release webview on documented reference hardware, require displayed FPS within 5% of the selected rate over 60 seconds for prepared representative fixtures at 8, 12, 24, and 30 FPS, within the display's refresh limit. Report interval percentiles and missed deadlines to detect bursts hidden by an average. Preserve the project's existing minimum of 8 steps/s at 0.25° with pre-rendered frames.

   For a fully prepared loop that fits the budget, require zero buffering and no new field-tile transfers, uploads, or full render-queue rebuilds during steady playback. Verify the first play after preparation as well as repeated loops. For a longer-than-96-step timeline and an over-budget timeline, verify bounded memory, retained key metadata, protected current frames, and sustainable streaming on the benchmark fixture; report genuine throughput limits explicitly.

   Integration tests must cover edit invalidation, pan/zoom/resize, project switching, shortening the timeline, macro-preview boundaries, failed transfers with bounded retries, and obsolete asynchronous responses. Use valid viewport addresses and time-varying scenes. Run the relevant UI and Rust suites plus UI typechecking; use real-app presentation measurements as the performance acceptance check.

Implement in reviewable stages: diagnostics and clock correction first; background/readiness scheduling next; preparation and cache coordination next; then any presentation, scene-sharing, or panel changes justified by the measurements. Update `spec.md` sections 9.4–9.5 to describe the final timing, readiness, and memory behavior and remove the current conflicting fixed-cache assumptions.

**Execution results**

The clock, preparation coordinator, readiness poller, protected tile cache, shared scene/key cache, and presentation acknowledgment are implemented. The release WebDriver fixture now resets playback explicitly for each rate and records bounded diagnostics. On the reference Mac, a prepared 24-step loop ran sequentially for 60 seconds at 8.000, 12.000, 23.954, and 29.851 displayed steps/s; no tile transfers or buffering occurred during those runs. A 120-step fixture exceeded the 1 GiB texture reservation and measured about 5.4 steps/s at 8 steps/s, with sustained tile transfers and evictions. That result is reported as a bounded streaming-throughput limit rather than presented as a fully resident-loop guarantee.
