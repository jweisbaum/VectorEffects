//! What a client is told before it sees a single tool (spec.md 8.8).
//!
//! A tool's description says what the tool does; nothing in forty-nine of
//! them says which to reach for. Asked for "a GRIB of the largest hurricane
//! of 2024", an agent given only the descriptions looked the storm up,
//! drew a vortex by hand at its landfall and exported that — and never
//! considered `import_history`, whose name and description read as plumbing.
//! This is the part of the interface that routes a request, so it is written
//! as instructions to the agent and tested the way it is used: by handing a
//! fresh agent a sentence and reading what it did (`tools/mcp-scenarios`).
//!
//! Keep it short enough to be read every time — it is in the client's
//! context for the whole session — and keep every name in it real:
//! `tests/mcp.rs` checks each tool and option it mentions still exists.

/// The server's `instructions`, sent once at initialisation: today's date,
/// then [`INSTRUCTIONS`].
///
/// The date leads because a model's sense of "now" is its training's. Asked
/// in September 2026 for the weather of "the last Newport Bermuda Race", a
/// test agent searched for 2025, settled on June 2024 and explained that the
/// 2026 race was still to come — with a tool result giving the date in front
/// of it, which it had asked for in the same breath as the search.
pub fn instructions(now: chrono::DateTime<chrono::Utc>) -> String {
    format!(
        "Today is {} (UTC). \"The last\", \"the latest\", \"this year's\" count back from today: before you search for an event's dates, work out from today's date which occurrence is the most recent one already over — an event held every two years has probably been held since the one you remember — and search for that year by number.\n\n{INSTRUCTIONS}",
        now.format("%A %-d %B %Y")
    )
}

/// The part of the instructions that does not change from day to day.
pub const INSTRUCTIONS: &str = "\
VectorEffects makes global wind and ocean-current fields and exports them as GRIB2 (export_grib) or Zarr (export_zarr). There are two ways to get a field. Decide which the request is before doing anything else.

1. REAL PAST WEATHER — a named hurricane or storm, a race or regatta, a voyage, an event, \"the weather on/during ...\". Download it; never draw weather that actually happened.
   a. history_archives, before anything else: the dates each archive covers, and today's date again.
   b. Find the event's UTC dates. Search the web if you can, otherwise ask. Cover the whole event: a storm from formation to dissipation, a race from its start to the last finisher.
   c. project_new: field_kind \"wind\", resolution \"0.25\" (the archives' own), step_hours 6 for an event of several days, 1 or 3 for a day or two. step_count can be 1: the next call sets it.
   d. import_history with fields [\"wind\"] (add \"current\" only if ocean current is wanted) and start/end as ISO 8601 UTC. It sizes the timeline to the range, downloads every step and stamps the timeline with the real dates.
   e. export_grib with an absolute path. The reference time is taken from the project.

2. INVENTED WEATHER — \"create / make / draw\" a storm, a front, a wind shift, a current. Draw it with object_create and animate it.
   a. project_new with enough steps for the event: a storm crossing an ocean basin takes 4 to 6 days, so step_hours 6 and step_count 17 to 25. Steps run from 0 to step_count - 1.
   b. A cyclone — tropical storm, hurricane, typhoon, low — that moves: storm_create. It draws the storm with the circle tool as ONE `circle` object turning the way its hemisphere turns (counter-clockwise in the north), keyed along the track you give, its peak wind and diameter running from the start values to the end values. A storm INTENSIFIES along its track unless the request says otherwise: peak_wind_end_mps higher than peak_wind_start_mps, inside its class. Peak wind by class, in m/s: tropical depression under 17, tropical storm 17 to 32, hurricane 33 and over.
   c. Anything else — a front, a jet, a wind shift, a current, a calm patch: tool_catalogue with the tool's name, then object_create.
   d. To animate any object: object_set with auto_key true at each step that matters, giving every property that changes there. On a gradient `circle`, SpeedMin is the wind at the CENTRE and SpeedMax the wind at the RIM, whatever the names suggest.
   e. Check with field_sample (numbers) or screenshot (picture), then export_grib if a file was asked for; a drawn project has no dates, so give it a reference time.

Conventions: positions are [lon, lat] in degrees, longitude -180 to 180. Speeds are m/s (1 kt = 0.514 m/s). A direction is the azimuth the flow moves TOWARD, degrees clockwise from north — the opposite of a meteorological \"from\" bearing. Times are UTC. File paths must be absolute or begin with ~/. A choice option is {\"kind\":\"choice\",\"index\":i} indexing the option's variants. Every edit is undoable (undo; history_list is that undo list, not past weather) and shows on the map as it is made.";
