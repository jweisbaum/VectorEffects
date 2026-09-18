//! What a client is told before it sees a single tool (spec.md 8.8).
//!
//! A tool's description says what the tool does; nothing in fifty of them
//! says which to reach for. Asked for "a GRIB of the largest hurricane of
//! 2024", an agent given only the descriptions looked the storm up, drew a
//! vortex by hand at its landfall and exported that — and never considered
//! `import_history`, whose name and description read as plumbing. This is
//! the part of the interface that routes a request, so it is written as
//! instructions to the agent and tested the way it is used: by handing a
//! fresh agent a sentence and reading what it did (`tools/mcp-scenarios`).
//!
//! **It is said twice, at two lengths, because no client can be relied on to
//! show the first.** Claude Code cuts a server's instructions at 2,048
//! characters — the first version of these ran to 3,450, and a session's
//! transcript shows them ending mid-sentence in step 2b, the conventions
//! never sent. Other clients are not known to show instructions at all. And
//! in a session that has a shell, the tools themselves are deferred behind a
//! search, so the opening lines of the instructions are all that argues for
//! using the application rather than writing Python. So [`INSTRUCTIONS`] is
//! short, leads with *when*, and fits; [`GUIDE`] is everything, and is what
//! the `vectoreffects_guide` tool returns to whoever asks, and what the
//! skill the application installs says (`mcp::skill`).
//!
//! Keep every name in both real: `tests/mcp.rs` checks each tool and option
//! they mention still exists.

use rmcp::handler::server::wrapper::Json;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Serialize;

use super::{ToolError, VectorEffects};

/// What Claude Code keeps of a server's instructions, in UTF-16 units; the
/// rest is replaced by "… [truncated]".
pub const INSTRUCTIONS_LIMIT: usize = 2048;

/// The line both texts open with.
///
/// The date leads because a model's sense of "now" is its training's. Asked
/// in September 2026 for the weather of "the last Newport Bermuda Race", a
/// test agent searched for 2025, settled on June 2024 and explained that the
/// 2026 race was still to come — with a tool result giving the date in front
/// of it, which it had asked for in the same breath as the search.
fn today(now: chrono::DateTime<chrono::Utc>) -> String {
    format!(
        "Today is {} (UTC). \"The last\" or \"the latest\" event counts back from today: work out which occurrence is the most recent one already over — one held every two years has probably been held since the one you remember — and search for that year by number.",
        now.format("%A %-d %B %Y")
    )
}

/// The server's `instructions`, sent once at initialisation: today's date,
/// then [`INSTRUCTIONS`]. Never longer than [`INSTRUCTIONS_LIMIT`].
pub fn instructions(now: chrono::DateTime<chrono::Utc>) -> String {
    format!("{}\n\n{INSTRUCTIONS}", today(now))
}

/// What `vectoreffects_guide` returns: today's date, then [`GUIDE`].
pub fn guide(now: chrono::DateTime<chrono::Utc>) -> String {
    format!("{}\n\n{GUIDE}", today(now))
}

/// When to use the application at all. It opens both texts, because it is
/// the part a model has to have read *before* it decides between these tools
/// and a script of its own. A macro only so that `concat!` can take it: it
/// is one literal, said once.
macro_rules! when {
    () => {
        "VectorEffects is an application open on this computer that makes global wind and ocean-current fields and exports them as GRIB2 (export_grib) or Zarr (export_zarr). USE THESE TOOLS whenever the user names VectorEffects, or wants a GRIB, a Zarr, or a wind, current, storm or weather field or file — for a router, a race, a voyage, a test. Do not write code to make or download such a file: these tools do it, and the user watches it appear on the map."
    };
}

/// The instructions proper: when to use the application, then the two routes in a line each.
pub const INSTRUCTIONS: &str = concat!(
    when!(),
    " Call vectoreffects_guide first for the full steps. In short, decide which of two requests it is:

1. REAL PAST WEATHER — a named hurricane or storm, a race, a voyage, \"the weather on/during ...\". Download it; never draw weather that actually happened. history_archives; find the event's UTC dates, all of it (search the web, else ask); project_new (field_kind \"wind\", resolution \"0.25\", step_hours 6 for several days, 1 or 3 for a day or two); import_history with start and end as ISO 8601 UTC; export_grib or export_zarr.

2. INVENTED WEATHER — \"create / make / draw\" a storm, a front, a wind shift, a current. project_new with enough steps (an ocean crossing takes 4 to 6 days: step_hours 6, step_count 17 to 25). A cyclone that moves: storm_create, one call; a storm INTENSIFIES along its track. Anything else: tool_catalogue, object_create, then object_set with auto_key true to animate. Check with field_sample or screenshot, then export with a reference time.

Conventions: positions are [lon, lat], longitude -180 to 180. Speeds are m/s (1 kt = 0.514). A direction is the azimuth the flow moves TOWARD, clockwise from north — the opposite of a meteorological \"from\" bearing. Times are UTC. File paths are absolute or begin with ~/."
);

/// Everything: the same opening, then each route step by step.
pub const GUIDE: &str = concat!(
    when!(),
    " There are two ways to get a field. Decide which the request is before doing anything else.

1. REAL PAST WEATHER — a named hurricane or storm, a race or regatta, a voyage, an event, \"the weather on/during ...\". Download it; never draw weather that actually happened.
   a. history_archives, before anything else: the dates each archive covers, and today's date again.
   b. Find the event's UTC dates. Search the web if you can, otherwise ask. Cover the whole event: a storm from formation to dissipation, a race from its start to the last finisher.
   c. project_new: field_kind \"wind\", resolution \"0.25\" (the archives' own), step_hours 6 for an event of several days, 1 or 3 for a day or two. step_count can be 1: the next call sets it.
   d. import_history with fields [\"wind\"] (add \"current\" only if ocean current is wanted) and start/end as ISO 8601 UTC. It sizes the timeline to the range, downloads every step and stamps the timeline with the real dates.
   e. export_grib or export_zarr with an absolute path. The reference time is taken from the project.

2. INVENTED WEATHER — \"create / make / draw\" a storm, a front, a wind shift, a current. Draw it with object_create and animate it.
   a. project_new with enough steps for the event: a storm crossing an ocean basin takes 4 to 6 days, so step_hours 6 and step_count 17 to 25. Steps run from 0 to step_count - 1.
   b. A cyclone — tropical storm, hurricane, typhoon, low — that moves: storm_create. It draws the storm with the circle tool as ONE `circle` object turning the way its hemisphere turns (counter-clockwise in the north), keyed along the track you give, its peak wind and diameter running from the start values to the end values. A storm INTENSIFIES along its track unless the request says otherwise: peak_wind_end_mps higher than peak_wind_start_mps, inside its class. Peak wind by class, in m/s: tropical depression under 17, tropical storm 17 to 32, hurricane 33 and over.
   c. Anything else — a front, a jet, a wind shift, a current, a calm patch: tool_catalogue with the tool's name, then object_create.
   d. To animate any object: object_set with auto_key true at each step that matters, giving every property that changes there. On a gradient `circle`, SpeedMin is the wind at the CENTRE and SpeedMax the wind at the RIM, whatever the names suggest.
   e. Check with field_sample (numbers) or screenshot (picture), then export_grib or export_zarr if a file was asked for; a drawn project has no dates, so give it a reference time.

A project that is already open may hold the user's unsaved work: project_status says, and project_new refuses to discard it unless told to. Ask, or project_save it first.

Conventions: positions are [lon, lat] in degrees, longitude -180 to 180. Speeds are m/s (1 kt = 0.514 m/s). A direction is the azimuth the flow moves TOWARD, degrees clockwise from north — the opposite of a meteorological \"from\" bearing. Times are UTC. File paths must be absolute or begin with ~/. A choice option is {\"kind\":\"choice\",\"index\":i} indexing the option's variants. Every edit is undoable (undo; history_list is that undo list, not past weather) and shows on the map as it is made."
);

/// What `vectoreffects_guide` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Guide {
    /// How to use every other tool: which route a request takes, step by
    /// step, and the conventions every value follows.
    pub guide: String,
}

#[tool_router(router = tool_router_guide, vis = "pub(crate)")]
impl<R: tauri::Runtime> VectorEffects<R> {
    #[tool(
        description = "START HERE. How to use VectorEffects, the application open on this computer that makes global wind and ocean-current fields and exports them as GRIB2 or Zarr. Call this first whenever the user names VectorEffects, or wants a GRIB, a Zarr, or a wind, current, storm or weather field or file — real past weather (a named hurricane, a race, a date) or invented weather (a storm to draw) — and before writing any code to make one: these tools already do it. Returns today's date, which tools to call in which order for each kind of request, and the conventions for positions, speeds, directions and paths. Reads nothing and changes nothing."
    )]
    async fn vectoreffects_guide(&self) -> std::result::Result<Json<Guide>, ToolError> {
        self.run("vectoreffects_guide", |_| {
            Ok(Guide {
                guide: guide(chrono::Utc::now()),
            })
        })
        .await
        .map(Json)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    /// The reference is the client, not this file: a Claude Code transcript
    /// of 2026-09-18 holds these instructions as it received them, cut at
    /// 2,048 with "… [truncated]". The date is the longest one there is.
    #[test]
    fn the_instructions_fit_what_claude_code_keeps() {
        let longest = chrono::Utc
            .with_ymd_and_hms(2026, 9, 30, 0, 0, 0)
            .single()
            .expect("a date");
        let text = instructions(longest);
        assert!(text.contains("Wednesday 30 September 2026"), "{text:.60}");
        let units = text.encode_utf16().count();
        assert!(
            units <= INSTRUCTIONS_LIMIT,
            "{units} units: Claude Code drops everything past {INSTRUCTIONS_LIMIT}"
        );
    }

    /// Whatever else is cut, the part that says when to use the application
    /// is inside the first quarter of what is kept.
    #[test]
    fn when_to_use_it_comes_before_how() {
        let text = instructions(chrono::Utc::now());
        let at = text.find("USE THESE TOOLS").expect("the routing sentence");
        assert!(at < INSTRUCTIONS_LIMIT / 4, "it starts at {at}");
        assert!(
            text.find("vectoreffects_guide").expect("the guide")
                < text.find("1. REAL").expect("routes")
        );
    }
}
