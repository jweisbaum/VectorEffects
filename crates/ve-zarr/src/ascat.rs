//! Scatterometer wind from Metop-B and Metop-C, the day's passes merged
//! (spec.md 4.10).
//!
//! ASCAT is a swath instrument: each satellite sees a band of ocean on its
//! ascending pass and another on its descending one, and Copernicus Marine
//! publishes the four as four daily datasets with gaps where nothing was
//! seen. One layer per dataset would be four half-empty layers of the same
//! wind; this reads the four together and makes one field of each day, a
//! cell taking the most recent measurement where passes overlap and staying
//! empty where none passed.
//!
//! The merge happens on the store's own 0.25 degree grid, before the regrid,
//! so a node of the common grid is the mean of what was actually measured
//! around it and missing where nothing was.

use crate::arco::{ArcoSpec, ArcoStore};
use crate::error::{Result, ZarrError};
use crate::parallel::try_join;
use crate::regrid::{CellGrid, to_era5_grid};
use crate::source::{Field, FieldSource, Step, Variable, step_at_hour, steps_between};
use crate::time::Utc;

/// A pass's eastward wind, northward wind and per-cell measurement time.
pub type Pass = (Vec<f32>, Vec<f32>, Vec<f64>);

/// Merges the passes of one day cell by cell: the most recent measurement
/// wins, and a cell no pass measured is missing.
///
/// A pass whose time at a cell is NaN made no measurement there, whatever
/// its components say; this is what keeps a fill value from winning.
pub fn merge_passes(passes: &[Pass]) -> (Vec<f32>, Vec<f32>) {
    let len = passes.first().map_or(0, |(u, _, _)| u.len());
    debug_assert!(
        passes
            .iter()
            .all(|(u, v, t)| u.len() == len && v.len() == len && t.len() == len),
        "every pass is on the one grid"
    );
    let mut out_u = vec![f32::NAN; len];
    let mut out_v = vec![f32::NAN; len];
    let mut newest = vec![f64::NEG_INFINITY; len];
    for (u, v, when) in passes {
        for i in 0..len {
            let t = when[i];
            if t.is_finite() && t > newest[i] && u[i].is_finite() && v[i].is_finite() {
                newest[i] = t;
                out_u[i] = u[i];
                out_v[i] = v[i];
            }
        }
    }
    (out_u, out_v)
}

/// The spec every ASCAT dataset is read with.
pub const SPEC: ArcoSpec = ArcoSpec {
    name: "ASCAT",
    variable: Variable::Wind10m,
    u_path: "/eastward_wind",
    v_path: "/northward_wind",
    level: None,
    time_path: Some("/measurement_time"),
    // ASCAT is daily, but its days are merged in `AscatStore`, which files
    // them under their midnights itself.
    daily: false,
};

/// The four passes read together.
pub struct AscatStore {
    members: Vec<ArcoStore>,
    /// Every day any member has, as the hour of its midnight, sorted.
    days: Vec<i64>,
    grid: CellGrid,
}

impl std::fmt::Debug for AscatStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AscatStore")
            .field("members", &self.members.len())
            .field("days", &self.days.len())
            .finish()
    }
}

/// The midnight of the day an hour falls in.
fn midnight(hour: i64) -> i64 {
    hour.div_euclid(24) * 24
}

/// The days a member holds, each as its midnight.
///
/// Floored rather than matched exactly, so the midnights an import asks for
/// are found whatever hour the publisher stamps a day with.
pub fn days_of(members: &[&[i64]]) -> Vec<i64> {
    let mut days: Vec<i64> = members
        .iter()
        .flat_map(|hours| hours.iter().map(|&h| midnight(h)))
        .collect();
    days.sort_unstable();
    days.dedup();
    days
}

impl AscatStore {
    /// Reads the passes together, which must share one grid.
    pub fn new(members: Vec<ArcoStore>) -> Result<Self> {
        if members.is_empty() {
            return Err(ZarrError::Open("no ASCAT dataset to read".to_owned()));
        }
        let grid = members[0].native_grid();
        if let Some(odd) = members.iter().find(|m| m.native_grid() != grid) {
            return Err(ZarrError::Layout(format!(
                "the ASCAT datasets are not on one grid: {:?} against {:?}",
                odd.native_grid(),
                grid
            )));
        }
        let days = days_of(&members.iter().map(|m| m.hours()).collect::<Vec<_>>());
        Ok(Self {
            members,
            days,
            grid,
        })
    }

    /// The passes of one day, on the store's grid: every member that has
    /// the day, read two at a time.
    fn passes_of(&self, day_hour: i64) -> Result<Vec<Pass>> {
        let wanted: Vec<(&ArcoStore, Step)> = self
            .members
            .iter()
            .filter_map(|member| {
                let hours = member.hours();
                let at = hours.iter().position(|&h| midnight(h) == day_hour)?;
                Some((
                    member,
                    Step {
                        index: at as u64,
                        valid_time: Utc::from_hours_since_unix_epoch(hours[at]),
                    },
                ))
            })
            .collect();
        let mut passes = Vec::with_capacity(wanted.len());
        for pair in wanted.chunks(2) {
            match pair {
                [(a, sa), (b, sb)] => {
                    let (pa, pb) =
                        try_join(|| a.read_native_timed(sa), || b.read_native_timed(sb))?;
                    passes.push(pa);
                    passes.push(pb);
                }
                [(a, sa)] => passes.push(a.read_native_timed(sa)?),
                _ => {}
            }
        }
        passes
            .into_iter()
            .map(|(u, v, when)| {
                let when = when.ok_or_else(|| {
                    ZarrError::Layout("an ASCAT dataset has no measurement time".to_owned())
                })?;
                Ok((u, v, when))
            })
            .collect()
    }
}

impl FieldSource for AscatStore {
    fn name(&self) -> &'static str {
        "ASCAT"
    }

    fn variables(&self) -> Vec<Variable> {
        vec![Variable::Wind10m]
    }

    fn coverage(&self) -> Option<(Utc, Utc)> {
        Some((
            Utc::from_hours_since_unix_epoch(*self.days.first()?),
            Utc::from_hours_since_unix_epoch(*self.days.last()?),
        ))
    }

    fn step_at(&self, time: Utc) -> Option<Step> {
        step_at_hour(&self.days, time)
    }

    fn steps_in_range(&self, start: Utc, end: Utc) -> Result<Vec<Step>> {
        steps_between(&self.days, start, end, "ASCAT")
    }

    fn read_step(&self, step: &Step) -> Result<Vec<Field>> {
        let day = *self.days.get(step.index as usize).ok_or_else(|| {
            ZarrError::TimeRange(format!("step {} is past the last day", step.index))
        })?;
        let passes = self.passes_of(day)?;
        // A day every pass lists and none has written yet is an empty day,
        // for the reason `ArcoStore::read_step` gives.
        let (u, v) = merge_passes(&passes);
        Ok(vec![Field {
            variable: Variable::Wind10m,
            u: to_era5_grid(self.grid, &u),
            v: to_era5_grid(self.grid, &v),
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three cells: one seen by the first pass alone, one by both with the
    /// second later, one by neither. The later measurement wins, the single
    /// one stands, and the unseen cell is missing — not zero.
    #[test]
    fn the_most_recent_pass_wins_and_an_unseen_cell_is_missing() {
        let first = (
            vec![1.0, 2.0, f32::NAN],
            vec![0.0, 0.0, f32::NAN],
            vec![100.0, 200.0, f64::NAN],
        );
        let second = (
            vec![f32::NAN, 5.0, f32::NAN],
            vec![f32::NAN, 1.0, f32::NAN],
            vec![f64::NAN, 250.0, f64::NAN],
        );
        let (u, v) = merge_passes(&[first, second]);
        assert_eq!(u[0], 1.0);
        assert_eq!((u[1], v[1]), (5.0, 1.0));
        assert!(u[2].is_nan() && v[2].is_nan());
    }

    /// A pass whose time at a cell is the fill, or whose component is, made
    /// no measurement there however the other says.
    #[test]
    fn a_pass_with_no_time_or_no_value_does_not_win() {
        let timed = (vec![3.0], vec![0.0], vec![100.0]);
        let timeless = (vec![9.0], vec![9.0], vec![f64::NAN]);
        let valueless = (vec![f32::NAN], vec![0.0], vec![900.0]);
        let (u, _) = merge_passes(&[timed.clone(), timeless.clone(), valueless.clone()]);
        assert_eq!(u[0], 3.0);
        let (u, _) = merge_passes(&[timeless, valueless]);
        assert!(u[0].is_nan());
        assert_eq!(merge_passes(&[]).0.len(), 0);
    }

    /// A day one satellite has and the other lacks is a day; a day both have
    /// is one day; and a day stamped at noon is the same day as its midnight.
    #[test]
    fn the_days_are_the_union_of_the_members_days() {
        let a = [0, 24, 48];
        let b = [24, 72 + 12];
        assert_eq!(days_of(&[&a, &b]), vec![0, 24, 48, 72]);
        assert_eq!(days_of(&[]), Vec::<i64>::new());
    }
}
