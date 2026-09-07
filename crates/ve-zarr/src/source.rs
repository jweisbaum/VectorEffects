//! What the export pipeline needs from a data source, whichever store it is.
//!
//! Every source hands back fields on the same grid -- the ERA5 0.25 degree
//! grid, 1440 x 721, north to south from the pole and east from the prime
//! meridian -- so the encoder never needs to know where a field came from,
//! and a wind field and a current field for the same hour can share a file
//! with an identical grid definition.

use crate::error::{Result, ZarrError};
use crate::time::Utc;

/// Points along a parallel of the common grid.
pub const NI: u64 = 1440;
/// Points along a meridian of the common grid.
pub const NJ: u64 = 721;
/// Values in one field on the common grid.
pub const POINTS_PER_STEP: usize = (NI * NJ) as usize;

/// One time step of an export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    /// Index meaningful only to the source that produced it.
    pub index: u64,
    /// The instant this step is valid at.
    pub valid_time: Utc,
}

/// A physical quantity a source can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Variable {
    /// 10 m wind, u and v.
    Wind10m,
    /// Total surface current, u and v.
    SurfaceCurrent,
}

/// A vector field for one step, on the common grid. NaN marks a missing
/// point.
#[derive(Debug, Clone)]
pub struct Field {
    /// What the field is.
    pub variable: Variable,
    /// Eastward component, in GRIB scanning order.
    pub u: Vec<f32>,
    /// Northward component, in GRIB scanning order.
    pub v: Vec<f32>,
}

/// A store that can be walked hour by hour.
pub trait FieldSource: Send + Sync {
    /// Short name, for logs and messages.
    fn name(&self) -> &'static str;

    /// The variables every step carries, in the order they are written.
    fn variables(&self) -> Vec<Variable>;

    /// The first and last valid times available.
    fn coverage(&self) -> Option<(Utc, Utc)>;

    /// The first hour whose data is preliminary and may be revised upstream,
    /// if the source has such a boundary; everything before it is final.
    ///
    /// Both stores carry one. ERA5's final stream trails real time by two to
    /// three months and ERA5T fills the rest; GlobCurrent's reprocessed `my`
    /// dataset stops a few months back and `nrt` fills the rest. Those hours
    /// export like any other -- the point is that a file built from them is
    /// not reproducible in the way one built from the final streams is, so
    /// the UI says where they begin.
    fn provisional_from(&self) -> Option<Utc> {
        None
    }

    /// The step valid at exactly `time`, if the source has one.
    fn step_at(&self, time: Utc) -> Option<Step>;

    /// Every step whose valid time falls in `[start, end]`, inclusive.
    fn steps_in_range(&self, start: Utc, end: Utc) -> Result<Vec<Step>>;

    /// Reads every field for one step.
    fn read_step(&self, step: &Step) -> Result<Vec<Field>>;
}

/// Selects the steps of a sorted hour list that fall in a range, as a helper
/// for sources that keep their time axis as hours since the Unix epoch.
pub fn steps_between(times: &[i64], start: Utc, end: Utc, what: &str) -> Result<Vec<Step>> {
    if start > end {
        return Err(ZarrError::TimeRange(format!(
            "the start time {} is after the end time {}",
            start.to_iso(),
            end.to_iso()
        )));
    }
    let (lo, hi) = (start.hours_since_unix_epoch(), end.hours_since_unix_epoch());
    let first = times.partition_point(|&t| t < lo);
    let last = times.partition_point(|&t| t <= hi);
    if first >= last {
        let coverage = match (times.first(), times.last()) {
            (Some(&a), Some(&b)) => format!(
                "it covers {} to {}",
                Utc::from_hours_since_unix_epoch(a).to_iso(),
                Utc::from_hours_since_unix_epoch(b).to_iso()
            ),
            _ => "the store is empty".to_string(),
        };
        return Err(ZarrError::TimeRange(format!(
            "no {what} step falls between {} and {}; {coverage}",
            start.to_iso(),
            end.to_iso()
        )));
    }
    Ok((first..last)
        .map(|index| Step {
            index: index as u64,
            valid_time: Utc::from_hours_since_unix_epoch(times[index]),
        })
        .collect())
}

/// Finds the step at exactly one hour in a sorted hour list.
pub fn step_at_hour(times: &[i64], time: Utc) -> Option<Step> {
    let h = time.hours_since_unix_epoch();
    times.binary_search(&h).ok().map(|index| Step {
        index: index as u64,
        valid_time: time,
    })
}

/// Several sources walked together: only hours every one of them has.
///
/// Its steps are numbered by position in the combined list, and each read
/// looks up the member sources by valid time, so no member's own indexing
/// leaks out.
pub struct Combined {
    members: Vec<Box<dyn FieldSource>>,
}

impl std::fmt::Debug for Combined {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Combined")
            .field(
                "members",
                &self.members.iter().map(|m| m.name()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Combined {
    /// Combines sources. At least one is required.
    pub fn new(members: Vec<Box<dyn FieldSource>>) -> Result<Self> {
        if members.is_empty() {
            return Err(ZarrError::Layout(
                "a combined source needs at least one member".into(),
            ));
        }
        Ok(Self { members })
    }
}

impl FieldSource for Combined {
    fn name(&self) -> &'static str {
        "combined"
    }

    fn variables(&self) -> Vec<Variable> {
        self.members.iter().flat_map(|m| m.variables()).collect()
    }

    fn coverage(&self) -> Option<(Utc, Utc)> {
        let mut it = self.members.iter().map(|m| m.coverage());
        let (mut lo, mut hi) = it.next()??;
        for c in it {
            let (a, b) = c?;
            lo = lo.max(a);
            hi = hi.min(b);
        }
        (lo <= hi).then_some((lo, hi))
    }

    fn provisional_from(&self) -> Option<Utc> {
        // A combined file is preliminary from the earliest hour any member is,
        // reported inside what the combination actually covers.
        let earliest = self
            .members
            .iter()
            .filter_map(|m| m.provisional_from())
            .min()?;
        let (lo, hi) = self.coverage()?;
        (earliest <= hi).then(|| earliest.max(lo))
    }

    fn step_at(&self, time: Utc) -> Option<Step> {
        self.members
            .iter()
            .all(|m| m.step_at(time).is_some())
            .then_some(Step {
                index: 0,
                valid_time: time,
            })
    }

    fn steps_in_range(&self, start: Utc, end: Utc) -> Result<Vec<Step>> {
        // Start from the first member's steps and keep the hours every other
        // member also has.
        let mut times: Vec<Utc> = self.members[0]
            .steps_in_range(start, end)?
            .into_iter()
            .map(|s| s.valid_time)
            .collect();
        for member in &self.members[1..] {
            times.retain(|&t| member.step_at(t).is_some());
        }
        if times.is_empty() {
            let names: Vec<_> = self.members.iter().map(|m| m.name()).collect();
            return Err(ZarrError::TimeRange(format!(
                "no hour between {} and {} is present in every source ({})",
                start.to_iso(),
                end.to_iso(),
                names.join(", ")
            )));
        }
        Ok(times
            .into_iter()
            .enumerate()
            .map(|(index, valid_time)| Step {
                index: index as u64,
                valid_time,
            })
            .collect())
    }

    fn read_step(&self, step: &Step) -> Result<Vec<Field>> {
        let mut fields = Vec::new();
        for member in &self.members {
            let own = member.step_at(step.valid_time).ok_or_else(|| {
                ZarrError::TimeRange(format!(
                    "{} has no step at {}",
                    member.name(),
                    step.valid_time.to_iso()
                ))
            })?;
            fields.extend(member.read_step(&own)?);
        }
        Ok(fields)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hours(from: Utc, n: i64) -> Vec<i64> {
        let base = from.hours_since_unix_epoch();
        (0..n).map(|i| base + i).collect()
    }

    #[test]
    fn a_range_selects_inclusive_hours() {
        let t0 = Utc {
            year: 2020,
            month: 1,
            day: 1,
            hour: 0,
        };
        let times = hours(t0, 48);
        let steps = steps_between(
            &times,
            Utc {
                year: 2020,
                month: 1,
                day: 1,
                hour: 5,
            },
            Utc {
                year: 2020,
                month: 1,
                day: 1,
                hour: 7,
            },
            "test",
        )
        .expect("selects");
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].index, 5);
        assert_eq!(steps[2].valid_time.hour, 7);
    }

    #[test]
    fn a_range_outside_the_axis_is_refused_with_the_coverage() {
        let times = hours(
            Utc {
                year: 2020,
                month: 1,
                day: 1,
                hour: 0,
            },
            24,
        );
        let far = Utc {
            year: 2030,
            month: 1,
            day: 1,
            hour: 0,
        };
        let err = steps_between(&times, far, far, "test").expect_err("refuses");
        assert!(format!("{err}").contains("2020-01-01T00:00Z"), "{err}");
    }

    #[test]
    fn an_inverted_range_is_refused() {
        let times = hours(
            Utc {
                year: 2020,
                month: 1,
                day: 1,
                hour: 0,
            },
            24,
        );
        let a = Utc {
            year: 2020,
            month: 1,
            day: 1,
            hour: 3,
        };
        let b = Utc {
            year: 2020,
            month: 1,
            day: 1,
            hour: 1,
        };
        assert!(steps_between(&times, a, b, "test").is_err());
    }

    #[test]
    fn step_at_finds_only_exact_hours() {
        let t0 = Utc {
            year: 2020,
            month: 1,
            day: 1,
            hour: 0,
        };
        let times: Vec<i64> = hours(t0, 24).into_iter().step_by(6).collect();
        assert_eq!(
            step_at_hour(&times, Utc { hour: 6, ..t0 }).map(|s| s.index),
            Some(1)
        );
        assert_eq!(step_at_hour(&times, Utc { hour: 7, ..t0 }), None);
    }

    /// A stand-in source with a fixed hour list.
    struct Fake {
        name: &'static str,
        times: Vec<i64>,
        provisional: Option<Utc>,
    }

    impl FieldSource for Fake {
        fn name(&self) -> &'static str {
            self.name
        }
        fn variables(&self) -> Vec<Variable> {
            vec![Variable::Wind10m]
        }
        fn coverage(&self) -> Option<(Utc, Utc)> {
            Some((
                Utc::from_hours_since_unix_epoch(*self.times.first()?),
                Utc::from_hours_since_unix_epoch(*self.times.last()?),
            ))
        }
        fn provisional_from(&self) -> Option<Utc> {
            self.provisional
        }
        fn step_at(&self, time: Utc) -> Option<Step> {
            step_at_hour(&self.times, time)
        }
        fn steps_in_range(&self, start: Utc, end: Utc) -> Result<Vec<Step>> {
            steps_between(&self.times, start, end, self.name)
        }
        fn read_step(&self, step: &Step) -> Result<Vec<Field>> {
            Ok(vec![Field {
                variable: Variable::Wind10m,
                u: vec![step.index as f32; 1],
                v: vec![0.0; 1],
            }])
        }
    }

    /// Wind and current lag real time by different amounts, so a combined
    /// export must stop where the shorter one does.
    #[test]
    fn a_combined_source_keeps_only_shared_hours() {
        let t0 = Utc {
            year: 2020,
            month: 1,
            day: 1,
            hour: 0,
        };
        let a = Fake {
            name: "a",
            times: hours(t0, 10),
            provisional: None,
        };
        let b = Fake {
            name: "b",
            times: hours(Utc { hour: 4, ..t0 }, 10),
            provisional: None,
        };
        let both = Combined::new(vec![Box::new(a), Box::new(b)]).expect("builds");

        assert_eq!(
            both.coverage(),
            Some((Utc { hour: 4, ..t0 }, Utc { hour: 9, ..t0 }))
        );
        let steps = both
            .steps_in_range(t0, Utc { hour: 23, ..t0 })
            .expect("selects");
        assert_eq!(steps.len(), 6);
        assert_eq!(steps[0].valid_time.hour, 4);
        assert_eq!(steps[5].valid_time.hour, 9);

        // Each read consults every member by valid time, not by index.
        let fields = both.read_step(&steps[0]).expect("reads");
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].u[0], 4.0, "a's own index for hour 4");
        assert_eq!(fields[1].u[0], 0.0, "b's own index for hour 4");
    }

    /// A file is only as final as its least final member, and the boundary
    /// is reported inside the hours the combination actually covers.
    #[test]
    fn a_combined_source_takes_the_earliest_provisional_hour() {
        let t0 = Utc {
            year: 2020,
            month: 1,
            day: 1,
            hour: 0,
        };
        let wind = Fake {
            name: "wind",
            times: hours(t0, 10),
            provisional: Some(Utc { hour: 8, ..t0 }),
        };
        let current = Fake {
            name: "current",
            times: hours(Utc { hour: 4, ..t0 }, 10),
            provisional: Some(Utc { hour: 6, ..t0 }),
        };
        let both = Combined::new(vec![Box::new(wind), Box::new(current)]).expect("builds");
        assert_eq!(both.provisional_from(), Some(Utc { hour: 6, ..t0 }));

        // A member whose provisional hours start before the shared window
        // reports the window's own first hour, not an hour outside it.
        let early = Fake {
            name: "wind",
            times: hours(t0, 10),
            provisional: Some(Utc { hour: 1, ..t0 }),
        };
        let late = Fake {
            name: "current",
            times: hours(Utc { hour: 4, ..t0 }, 10),
            provisional: None,
        };
        let both = Combined::new(vec![Box::new(early), Box::new(late)]).expect("builds");
        assert_eq!(both.provisional_from(), Some(Utc { hour: 4, ..t0 }));
    }

    #[test]
    fn a_combined_source_with_no_overlap_is_refused_clearly() {
        let t0 = Utc {
            year: 2020,
            month: 1,
            day: 1,
            hour: 0,
        };
        let a = Fake {
            name: "wind",
            times: hours(t0, 3),
            provisional: None,
        };
        let b = Fake {
            name: "current",
            times: hours(Utc { hour: 12, ..t0 }, 3),
            provisional: None,
        };
        let both = Combined::new(vec![Box::new(a), Box::new(b)]).expect("builds");
        assert_eq!(both.coverage(), None);
        let err = both
            .steps_in_range(t0, Utc { hour: 23, ..t0 })
            .expect_err("refuses");
        assert!(format!("{err}").contains("wind, current"), "{err}");
    }
}
