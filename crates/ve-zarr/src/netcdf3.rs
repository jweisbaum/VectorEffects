//! A reader for NetCDF-3 — the classic and 64-bit-offset formats.
//!
//! What a NOAA ERDDAP server answers a subset request with (spec.md 4.10):
//! a header naming the dimensions, the attributes and the variables, then
//! each variable's values, big-endian, at the offset the header gives. That
//! is the whole format; there is no compression and no HDF5 inside it, which
//! is why it can be read here in a few hundred lines rather than linked from
//! a C library (invariant 5, the three-platform build).
//!
//! Reads only. Record variables — those along an unlimited dimension — are
//! read too, interleaved as the format lays them out.

use crate::error::{Result, ZarrError};

/// One attribute's value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Text.
    Text(String),
    /// Numbers, of whatever type they were stored as.
    Numbers(Vec<f64>),
}

/// A variable, as the header describes it.
#[derive(Debug, Clone)]
pub struct Variable {
    /// Its name.
    pub name: String,
    /// Indices into the file's dimensions, outermost first.
    pub dimensions: Vec<usize>,
    /// Its attributes, in the order they were written.
    pub attributes: Vec<(String, Value)>,
    kind: Kind,
    /// Bytes from the start of the file to the first value.
    begin: u64,
    /// Whether it runs along the unlimited dimension.
    record: bool,
}

impl Variable {
    /// A text attribute.
    pub fn text(&self, name: &str) -> Option<&str> {
        self.attributes.iter().find_map(|(n, v)| match v {
            Value::Text(text) if n == name => Some(text.as_str()),
            _ => None,
        })
    }

    /// The first number of a numeric attribute.
    pub fn number(&self, name: &str) -> Option<f64> {
        self.attributes.iter().find_map(|(n, v)| match v {
            Value::Numbers(numbers) if n == name => numbers.first().copied(),
            _ => None,
        })
    }
}

/// How a value is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Byte,
    Char,
    Short,
    Int,
    Float,
    Double,
}

impl Kind {
    fn of(code: u32) -> Result<Self> {
        Ok(match code {
            1 => Self::Byte,
            2 => Self::Char,
            3 => Self::Short,
            4 => Self::Int,
            5 => Self::Float,
            6 => Self::Double,
            other => return Err(layout(format!("an unknown value type {other}"))),
        })
    }

    fn size(self) -> usize {
        match self {
            Self::Byte | Self::Char => 1,
            Self::Short => 2,
            Self::Int | Self::Float => 4,
            Self::Double => 8,
        }
    }

    fn decode(self, bytes: &[u8]) -> f64 {
        match self {
            Self::Byte => f64::from(bytes[0] as i8),
            Self::Char => f64::from(bytes[0]),
            Self::Short => f64::from(i16::from_be_bytes([bytes[0], bytes[1]])),
            Self::Int => f64::from(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
            Self::Float => f64::from(f32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
            Self::Double => f64::from_be_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]),
        }
    }
}

/// A NetCDF-3 file held in memory.
#[derive(Debug, Clone)]
pub struct File<'a> {
    bytes: &'a [u8],
    /// The dimensions, as `(name, length)`; length 0 is the unlimited one.
    pub dimensions: Vec<(String, usize)>,
    /// The variables, in the order they were written.
    pub variables: Vec<Variable>,
    /// How many records the unlimited dimension holds.
    records: usize,
}

fn layout(why: String) -> ZarrError {
    ZarrError::Layout(format!("not a readable NetCDF-3 file: {why}"))
}

/// A cursor over the header.
struct Header<'a> {
    bytes: &'a [u8],
    at: usize,
    wide_offsets: bool,
}

impl Header<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self
            .at
            .checked_add(n)
            .filter(|end| *end <= self.bytes.len());
        let end = end.ok_or_else(|| layout("the header stops short".to_owned()))?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn count(&mut self) -> Result<usize> {
        // Every count is followed by that many things of at least four bytes,
        // so a count larger than the file is a corrupt header, refused before
        // it reaches an allocation.
        let n = self.u32()? as usize;
        if n > self.bytes.len() {
            return Err(layout(format!(
                "a count of {n} in a file of {} bytes",
                self.bytes.len()
            )));
        }
        Ok(n)
    }

    fn padded(&mut self, n: usize) -> Result<&[u8]> {
        let pad = (4 - n % 4) % 4;
        let start = self.at;
        self.take(n + pad)?;
        Ok(&self.bytes[start..start + n])
    }

    fn name(&mut self) -> Result<String> {
        let n = self.count()?;
        let raw = self.padded(n)?;
        String::from_utf8(raw.to_vec()).map_err(|_| layout("a name that is not UTF-8".to_owned()))
    }

    /// A list's tag and length; an absent list is written as two zeros.
    fn list(&mut self, tag: u32, what: &str) -> Result<usize> {
        let found = self.u32()?;
        let n = self.count()?;
        if found == 0 && n == 0 {
            return Ok(0);
        }
        if found != tag {
            return Err(layout(format!("the {what} list has tag {found}")));
        }
        Ok(n)
    }

    fn attributes(&mut self) -> Result<Vec<(String, Value)>> {
        let n = self.list(0x0C, "attribute")?;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let name = self.name()?;
            let kind = Kind::of(self.u32()?)?;
            let count = self.count()?;
            let raw = self.padded(count * kind.size())?;
            let value = if kind == Kind::Char {
                Value::Text(
                    String::from_utf8_lossy(raw)
                        .trim_end_matches('\0')
                        .to_owned(),
                )
            } else {
                Value::Numbers(raw.chunks(kind.size()).map(|c| kind.decode(c)).collect())
            };
            out.push((name, value));
        }
        Ok(out)
    }
}

impl<'a> File<'a> {
    /// Reads the header. The values are read on demand.
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.starts_with(b"\x89HDF") {
            return Err(layout(
                "this is NetCDF-4, which is HDF5 inside; ask the server for NetCDF-3".to_owned(),
            ));
        }
        let wide_offsets = match bytes.get(..4) {
            Some(b"CDF\x01") => false,
            Some(b"CDF\x02") => true,
            _ => return Err(layout("it does not begin with CDF".to_owned())),
        };
        let mut h = Header {
            bytes,
            at: 4,
            wide_offsets,
        };
        let records = h.u32()? as usize;

        let n = h.list(0x0A, "dimension")?;
        let mut dimensions = Vec::with_capacity(n);
        for _ in 0..n {
            let name = h.name()?;
            let length = h.u32()? as usize;
            dimensions.push((name, length));
        }
        h.attributes()?;

        let n = h.list(0x0B, "variable")?;
        let mut variables = Vec::with_capacity(n);
        for _ in 0..n {
            let name = h.name()?;
            let rank = h.count()?;
            let mut dims = Vec::with_capacity(rank);
            for _ in 0..rank {
                let d = h.u32()? as usize;
                if d >= dimensions.len() {
                    return Err(layout(format!("{name} names dimension {d}")));
                }
                dims.push(d);
            }
            let attributes = h.attributes()?;
            let kind = Kind::of(h.u32()?)?;
            let _vsize = h.u32()?;
            let begin = if h.wide_offsets {
                let b = h.take(8)?;
                u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
            } else {
                u64::from(h.u32()?)
            };
            let record = dims.first().is_some_and(|d| dimensions[*d].1 == 0);
            variables.push(Variable {
                name,
                dimensions: dims,
                attributes,
                kind,
                begin,
                record,
            });
        }
        Ok(Self {
            bytes,
            dimensions,
            variables,
            records,
        })
    }

    /// A variable by name.
    pub fn variable(&self, name: &str) -> Option<&Variable> {
        self.variables.iter().find(|v| v.name == name)
    }

    /// A variable's shape, with the unlimited dimension at its record count.
    pub fn shape(&self, variable: &Variable) -> Vec<usize> {
        variable
            .dimensions
            .iter()
            .map(|d| match self.dimensions[*d].1 {
                0 => self.records,
                n => n,
            })
            .collect()
    }

    /// Bytes one record of a record variable takes, before padding.
    fn record_bytes(&self, variable: &Variable) -> usize {
        let per: usize = self.shape(variable).iter().skip(1).product();
        per * variable.kind.size()
    }

    /// Every value of a variable, in its own order, as `f64`. The fill value
    /// is returned as stored; what it means is the caller's to say.
    pub fn read_f64(&self, name: &str) -> Result<Vec<f64>> {
        let variable = self
            .variable(name)
            .ok_or_else(|| layout(format!("there is no variable {name}")))?;
        let size = variable.kind.size();
        let short = || layout(format!("{name}'s values run past the end of the file"));
        let decode = |at: usize, bytes: usize| -> Result<Vec<f64>> {
            let end = at.checked_add(bytes).filter(|e| *e <= self.bytes.len());
            let raw = &self.bytes[at..end.ok_or_else(short)?];
            Ok(raw.chunks(size).map(|c| variable.kind.decode(c)).collect())
        };
        let begin = usize::try_from(variable.begin).map_err(|_| short())?;
        if !variable.record {
            let count: usize = self.shape(variable).iter().product();
            return decode(begin, count * size);
        }
        // Records are interleaved: each record holds one slab of every record
        // variable in turn, each padded to four bytes — unless there is only
        // one record variable, which the format leaves unpadded.
        let record_vars: Vec<&Variable> = self.variables.iter().filter(|v| v.record).collect();
        let stride: usize = if record_vars.len() == 1 {
            self.record_bytes(variable)
        } else {
            record_vars
                .iter()
                .map(|v| {
                    let n = self.record_bytes(v);
                    n + (4 - n % 4) % 4
                })
                .sum()
        };
        let one = self.record_bytes(variable);
        let mut out = Vec::with_capacity(self.records * one / size.max(1));
        for r in 0..self.records {
            out.extend(decode(begin + r * stride, one)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OISST: &[u8] = include_bytes!("../tests/fixtures/erddap/oisst-subset.nc");
    const OISST_CSV: &str = include_str!("../tests/fixtures/erddap/oisst-subset.csv");

    /// The same subset as the server wrote it in CSV: an independent decoding
    /// of every value, in the file's own order (latitude, then longitude).
    fn csv_values() -> Vec<(f64, f64, f64)> {
        OISST_CSV
            .lines()
            .skip(2)
            .map(|line| {
                let cells: Vec<&str> = line.split(',').collect();
                let number = |i: usize| cells[i].parse::<f64>().unwrap_or(f64::NAN);
                (number(2), number(3), number(4))
            })
            .collect()
    }

    #[test]
    fn the_header_names_what_the_server_sent() {
        let file = File::parse(OISST).expect("parses");
        let names: Vec<&str> = file.variables.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["time", "zlev", "latitude", "longitude", "sst"]);
        let sst = file.variable("sst").expect("sst");
        assert_eq!(file.shape(sst), vec![1, 1, 9, 9]);
        assert_eq!(sst.text("units"), Some("degree_C"));
        assert_eq!(
            file.variable("time").and_then(|t| t.text("units")),
            Some("seconds since 1970-01-01T00:00:00Z")
        );
        assert!((sst.number("_FillValue").expect("a fill") + 9.99).abs() < 1e-5);
    }

    /// Every value against the server's CSV of the same request, the fill
    /// as NaN — and the axes, and the time, which is 2026-09-01T12Z: 20 697
    /// days and twelve hours after 1970 (20 727 for 1 October, less 30).
    #[test]
    fn every_value_matches_the_server_s_own_csv() {
        let file = File::parse(OISST).expect("parses");
        let expected = csv_values();
        assert_eq!(expected.len(), 81);

        let lat = file.read_f64("latitude").expect("latitude");
        let lon = file.read_f64("longitude").expect("longitude");
        let sst = file.read_f64("sst").expect("sst");
        let fill = file
            .variable("sst")
            .and_then(|v| v.number("_FillValue"))
            .unwrap();
        assert_eq!(sst.len(), 81);
        for (i, (want_lat, want_lon, want)) in expected.iter().enumerate() {
            assert_eq!(lat[i / 9], *want_lat, "latitude {i}");
            assert_eq!(lon[i % 9], *want_lon, "longitude {i}");
            let got = if sst[i] == fill { f64::NAN } else { sst[i] };
            if want.is_nan() {
                assert!(got.is_nan(), "{i}: {got} where the CSV has NaN");
            } else {
                assert!((got - want).abs() < 1e-4, "{i}: {got} != {want}");
            }
        }
        assert_eq!(
            file.read_f64("time").expect("time"),
            vec![(20_697.0 * 86_400.0) + 12.0 * 3600.0]
        );
    }

    #[test]
    fn what_is_not_netcdf3_is_refused_by_name() {
        assert!(
            File::parse(b"\x89HDF\r\n\x1a\n")
                .unwrap_err()
                .to_string()
                .contains("HDF5")
        );
        assert!(File::parse(b"CDF").is_err());
        assert!(File::parse(&OISST[..200]).is_err(), "a truncated header");
        let mut short = OISST.to_vec();
        short.truncate(OISST.len() - 10);
        let file = File::parse(&short).expect("the header is whole");
        assert!(file.read_f64("sst").is_err(), "the data is not");
        assert!(File::parse(OISST).unwrap().read_f64("nothing").is_err());
    }
}
