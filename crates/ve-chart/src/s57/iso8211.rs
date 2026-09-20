//! The ISO/IEC 8211 container an S-57 cell is written in.
//!
//! A file is a run of records. Each opens with a 24-byte leader giving its
//! length and the sizes of its own directory entries, then a directory of
//! `(tag, length, position)`, then the fields those entries address. The
//! first record is the *data descriptive record*: it carries, for every tag
//! in the file, the names of that field's subfields and a format string
//! saying how to read them. Every later record is data, read through those
//! formats.
//!
//! Nothing here knows what S-57 is — this is the envelope, and `super`
//! reads the letter.

use std::collections::HashMap;

use crate::error::{ChartError, Result};

/// Field terminator.
const FT: u8 = 0x1e;
/// Unit (subfield) terminator.
const UT: u8 = 0x1f;

/// How one subfield is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    /// Characters: a fixed width, or up to the next unit terminator.
    Text(Option<usize>),
    /// An integer written as characters.
    Int(Option<usize>),
    /// A real written as characters.
    Real(Option<usize>),
    /// Raw bytes, from a bit width that is always a whole number of them.
    Bytes(usize),
    /// An unsigned integer of this many bytes, little-endian.
    Unsigned(usize),
    /// A signed integer of this many bytes, little-endian.
    Signed(usize),
}

/// A subfield's value.
#[derive(Debug, Clone, PartialEq)]
pub enum Sub {
    /// A number that was stored as one.
    Int(i64),
    /// A real.
    Real(f64),
    /// Characters, trimmed of padding.
    Text(String),
    /// Bytes, for the record names S-57 packs into bit strings.
    Bytes(Vec<u8>),
}

impl Sub {
    /// The value as an integer, where it is one or reads as one.
    pub fn int(&self) -> Option<i64> {
        match self {
            Self::Int(n) => Some(*n),
            Self::Real(x) => Some(*x as i64),
            Self::Text(text) => text.trim().parse().ok(),
            Self::Bytes(_) => None,
        }
    }

    /// The value as text, where it is text.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    /// The value's bytes, where it is a bit string.
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }
}

/// How one tag's field is laid out: its subfield names, in order, and how
/// each is stored. A field whose descriptor begins `*` repeats its whole run
/// of subfields until the field's bytes are used up.
#[derive(Debug, Clone)]
pub struct FieldFormat {
    /// Subfield names, in order.
    pub labels: Vec<String>,
    controls: Vec<Control>,
    repeats: bool,
}

impl FieldFormat {
    /// Reads a field's bytes into one row of values per repeat.
    ///
    /// A row short of its labels is kept: the last subfield of a record's
    /// last field is sometimes simply absent, and dropping the row would
    /// lose the coordinates in front of it.
    pub fn read(&self, mut data: &[u8]) -> Vec<Vec<Sub>> {
        // The field's own terminator is not part of its data.
        while data.last() == Some(&FT) || data.last() == Some(&UT) {
            data = &data[..data.len() - 1];
        }
        let mut rows = Vec::new();
        let mut at = 0;
        while at < data.len() {
            let mut row = Vec::with_capacity(self.controls.len());
            for control in &self.controls {
                match read_one(*control, data, &mut at) {
                    Some(value) => row.push(value),
                    None => break,
                }
            }
            if row.is_empty() {
                break;
            }
            rows.push(row);
            if !self.repeats {
                break;
            }
        }
        rows
    }

    /// One row, for the fields that hold exactly one.
    pub fn read_one(&self, data: &[u8]) -> Vec<Sub> {
        self.read(data).into_iter().next().unwrap_or_default()
    }

    /// The position of a named subfield within a row.
    pub fn index_of(&self, label: &str) -> Option<usize> {
        self.labels.iter().position(|name| name == label)
    }
}

fn read_one(control: Control, data: &[u8], at: &mut usize) -> Option<Sub> {
    if *at >= data.len() {
        return None;
    }
    let take = |at: &mut usize, n: usize| -> Option<&[u8]> {
        let end = *at + n;
        if end > data.len() {
            return None;
        }
        let out = &data[*at..end];
        *at = end;
        Some(out)
    };
    // A variable-width subfield runs to the next unit terminator, which is
    // consumed with it.
    let variable = |at: &mut usize| -> &[u8] {
        let start = *at;
        let end = data[start..]
            .iter()
            .position(|b| *b == UT)
            .map_or(data.len(), |n| start + n);
        *at = (end + 1).min(data.len().max(end));
        &data[start..end]
    };
    let text = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes)
            .trim_end_matches(['\0', ' '])
            .to_owned()
    };
    Some(match control {
        Control::Text(Some(n)) => Sub::Text(text(take(at, n)?)),
        Control::Text(None) => Sub::Text(text(variable(at))),
        Control::Int(width) => {
            let raw = match width {
                Some(n) => text(take(at, n)?),
                None => text(variable(at)),
            };
            Sub::Int(raw.trim().parse().unwrap_or(0))
        }
        Control::Real(width) => {
            let raw = match width {
                Some(n) => text(take(at, n)?),
                None => text(variable(at)),
            };
            Sub::Real(raw.trim().parse().unwrap_or(0.0))
        }
        Control::Bytes(n) => Sub::Bytes(take(at, n)?.to_vec()),
        Control::Unsigned(n) => {
            let bytes = take(at, n)?;
            let mut value = 0_u64;
            for (i, byte) in bytes.iter().enumerate() {
                value |= u64::from(*byte) << (8 * i);
            }
            Sub::Int(value as i64)
        }
        Control::Signed(n) => {
            let bytes = take(at, n)?;
            let mut value = 0_u64;
            for (i, byte) in bytes.iter().enumerate() {
                value |= u64::from(*byte) << (8 * i);
            }
            // Sign-extend from the width actually stored.
            let shift = 64 - 8 * n;
            Sub::Int(((value << shift) as i64) >> shift)
        }
    })
}

/// Splits a format string's top level on commas, keeping bracketed groups
/// whole: `2b11,3(A,I(5)),b14` is three items.
fn split_items(text: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut depth = 0;
    let mut current = String::new();
    for c in text.chars() {
        match c {
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 => items.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        items.push(current);
    }
    items
}

/// Turns one format item into the controls it stands for.
fn controls_of(item: &str, out: &mut Vec<Control>) -> Result<()> {
    let item = item.trim();
    if item.is_empty() {
        return Ok(());
    }
    // A leading count repeats what follows.
    let digits: String = item.chars().take_while(char::is_ascii_digit).collect();
    let rest = &item[digits.len()..];
    let repeat: usize = digits.parse().unwrap_or(1);
    if repeat == 0 || repeat > 1024 {
        return Err(ChartError::malformed(
            "an 8211 format",
            format!("repeat {repeat}"),
        ));
    }

    if let Some(inner) = rest.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        for _ in 0..repeat {
            for piece in split_items(inner) {
                controls_of(&piece, out)?;
            }
        }
        return Ok(());
    }

    // A width in brackets, where there is one.
    let (letters, width) = match rest.find('(') {
        Some(open) => {
            let close = rest
                .rfind(')')
                .ok_or_else(|| ChartError::malformed("an 8211 format", rest.to_owned()))?;
            (
                &rest[..open],
                rest[open + 1..close].trim().parse::<usize>().ok(),
            )
        }
        None => (rest, None),
    };

    let control = match letters.trim() {
        "A" => Control::Text(width),
        "I" => Control::Int(width),
        "R" => Control::Real(width),
        "B" => Control::Bytes(width.unwrap_or(0).div_ceil(8)),
        // `b` then a type digit then a width in bytes: b11 is one unsigned
        // byte, b24 four signed ones.
        binary if binary.starts_with('b') && binary.len() == 3 => {
            let kind = binary.as_bytes()[1];
            let bytes = usize::from(binary.as_bytes()[2] - b'0');
            if !(1..=8).contains(&bytes) {
                return Err(ChartError::malformed("an 8211 format", binary.to_owned()));
            }
            match kind {
                b'1' => Control::Unsigned(bytes),
                b'2' => Control::Signed(bytes),
                _ => Control::Bytes(bytes),
            }
        }
        // `X` is filler, and anything else is not something this reads.
        "X" => Control::Bytes(width.unwrap_or(1)),
        other => {
            return Err(ChartError::Unsupported(format!(
                "8211 subfield format {other:?}"
            )));
        }
    };
    for _ in 0..repeat {
        out.push(control);
    }
    Ok(())
}

fn field_format(descriptor: &str, controls: &str) -> Result<FieldFormat> {
    let repeats = descriptor.starts_with('*');
    let labels: Vec<String> = descriptor
        .trim_start_matches('*')
        .split('!')
        .filter(|label| !label.is_empty())
        .map(str::to_owned)
        .collect();
    let inner = controls
        .trim()
        .strip_prefix('(')
        .and_then(|text| text.strip_suffix(')'))
        .unwrap_or(controls.trim());
    let mut parsed = Vec::new();
    for item in split_items(inner) {
        controls_of(&item, &mut parsed)?;
    }
    Ok(FieldFormat {
        labels,
        controls: parsed,
        repeats,
    })
}

/// One record's fields, in the order the directory listed them.
#[derive(Debug, Clone)]
pub struct Record<'a> {
    /// `(tag, bytes)` per field.
    pub fields: Vec<(&'a str, &'a [u8])>,
}

impl<'a> Record<'a> {
    /// The first field with this tag.
    pub fn field(&self, tag: &str) -> Option<&'a [u8]> {
        self.fields
            .iter()
            .find(|(name, _)| *name == tag)
            .map(|(_, data)| *data)
    }

    /// Whether the record holds a field.
    pub fn has(&self, tag: &str) -> bool {
        self.field(tag).is_some()
    }
}

/// A leader's sizes, which differ per record.
#[derive(Debug, Clone, Copy)]
struct Leader {
    length: usize,
    base: usize,
    length_size: usize,
    position_size: usize,
    tag_size: usize,
}

fn digits(bytes: &[u8], what: &'static str) -> Result<usize> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ChartError::malformed(what, "not text"))?
        .trim();
    if text.is_empty() {
        return Ok(0);
    }
    text.parse()
        .map_err(|_| ChartError::malformed(what, format!("{text:?} is not a number")))
}

fn leader_of(data: &[u8]) -> Result<Leader> {
    if data.len() < 24 {
        return Err(ChartError::malformed("an 8211 leader", "under 24 bytes"));
    }
    let size = |at: usize| -> Result<usize> {
        let c = data[at];
        if c.is_ascii_digit() {
            Ok(usize::from(c - b'0'))
        } else {
            Err(ChartError::malformed("an 8211 leader", "bad entry map"))
        }
    };
    Ok(Leader {
        length: digits(&data[0..5], "an 8211 record length")?,
        base: digits(&data[12..17], "an 8211 field area address")?,
        length_size: size(20)?,
        position_size: size(21)?,
        tag_size: size(23)?,
    })
}

/// Splits one record into its fields.
fn record_of<'a>(data: &'a [u8], leader: Leader) -> Result<Record<'a>> {
    let mut fields = Vec::new();
    let mut at = 24;
    let entry = leader.tag_size + leader.length_size + leader.position_size;
    while at < leader.base && at < data.len() && data[at] != FT {
        if at + entry > data.len() {
            return Err(ChartError::malformed(
                "an 8211 directory",
                "runs past the record",
            ));
        }
        let tag = std::str::from_utf8(&data[at..at + leader.tag_size])
            .map_err(|_| ChartError::malformed("an 8211 tag", "not text"))?
            .trim();
        let length = digits(
            &data[at + leader.tag_size..at + leader.tag_size + leader.length_size],
            "an 8211 field length",
        )?;
        let start = digits(
            &data[at + leader.tag_size + leader.length_size..at + entry],
            "an 8211 field position",
        )?;
        let from = leader.base + start;
        let to = from + length;
        if to > data.len() {
            return Err(ChartError::malformed(
                "an 8211 field",
                "runs past the record",
            ));
        }
        fields.push((tag, &data[from..to]));
        at += entry;
    }
    Ok(Record { fields })
}

/// An 8211 file: the formats from its first record, and its data records.
#[derive(Debug)]
pub struct File<'a> {
    /// Every tag's layout, from the data descriptive record.
    pub formats: HashMap<String, FieldFormat>,
    data: &'a [u8],
    at: usize,
}

impl<'a> File<'a> {
    /// Reads the descriptive record and stops at the first data record.
    pub fn open(data: &'a [u8]) -> Result<Self> {
        let leader = leader_of(data)?;
        if leader.length == 0 || leader.length > data.len() {
            return Err(ChartError::malformed(
                "an 8211 file",
                "the first record's length is impossible",
            ));
        }
        let ddr = record_of(&data[..leader.length], leader)?;
        let mut formats = HashMap::new();
        for (tag, field) in &ddr.fields {
            // A descriptive field is: name, then the array descriptor, then
            // the format controls, separated by unit terminators.
            let parts: Vec<&[u8]> = field
                .strip_suffix(&[FT][..])
                .unwrap_or(field)
                .split(|b| *b == UT)
                .collect();
            let (Some(descriptor), Some(controls)) = (parts.get(1), parts.get(2)) else {
                continue;
            };
            let descriptor = String::from_utf8_lossy(descriptor).into_owned();
            let controls = String::from_utf8_lossy(controls).into_owned();
            // `0000` describes the file's own field tree and has no data.
            if *tag == "0000" {
                continue;
            }
            formats.insert((*tag).to_owned(), field_format(&descriptor, &controls)?);
        }
        Ok(Self {
            formats,
            data,
            at: leader.length,
        })
    }

    /// The layout of a tag's field.
    pub fn format(&self, tag: &str) -> Option<&FieldFormat> {
        self.formats.get(tag)
    }

    /// The next data record, or `None` at the end of the file.
    pub fn next_record(&mut self) -> Result<Option<Record<'a>>> {
        if self.at >= self.data.len() {
            return Ok(None);
        }
        // Trailing padding, rather than another record.
        if self.data.len() - self.at < 24 {
            self.at = self.data.len();
            return Ok(None);
        }
        let leader = leader_of(&self.data[self.at..])?;
        if leader.length < 24 || self.at + leader.length > self.data.len() {
            return Err(ChartError::malformed(
                "an 8211 record",
                format!("length {} at offset {}", leader.length, self.at),
            ));
        }
        let record = record_of(&self.data[self.at..self.at + leader.length], leader)?;
        self.at += leader.length;
        Ok(Some(record))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Written the long way round, so the test asserts the format and not a
    /// second copy of the writer: a two-field descriptive record and one
    /// data record holding a repeating pair of signed four-byte integers.
    fn synthetic() -> Vec<u8> {
        let ddr_fields: [(&str, &[u8]); 2] = [
            ("0001", b"0500;&   ISO 8211 Record\x1f\x1f(b12)\x1e"),
            ("SG2D", b"2500;&   2-D\x1f*YCOO!XCOO\x1f(2b24)\x1e"),
        ];
        let data_fields: [(&str, Vec<u8>); 2] = [
            ("0001", vec![1, 0, FT]),
            ("SG2D", {
                let mut out = Vec::new();
                for (y, x) in [(500_000_i32, -1_200_000_i32), (510_000, -1_190_000)] {
                    out.extend(y.to_le_bytes());
                    out.extend(x.to_le_bytes());
                }
                out.push(FT);
                out
            }),
        ];
        let mut out = Vec::new();
        for record in [
            ddr_fields
                .iter()
                .map(|(tag, data)| (*tag, data.to_vec()))
                .collect::<Vec<_>>(),
            data_fields.to_vec(),
        ] {
            // Directory first, so the field area's base address is known.
            let mut directory = Vec::new();
            let mut position = 0;
            for (tag, data) in &record {
                directory.extend(format!("{tag}{:03}{:04}", data.len(), position).bytes());
                position += data.len();
            }
            directory.push(FT);
            let base = 24 + directory.len();
            let length = base + position;
            // Length, then five bytes of version and application, two of
            // field control length, the field area's address at 12..17,
            // three reserved, and the entry map at 20..24.
            let leader = format!("{length:05}3LE 09 {base:05}   3404");
            assert_eq!(leader.len(), 24, "{leader:?}");
            out.extend(leader.bytes());
            out.extend(directory);
            for (_, data) in &record {
                out.extend(data);
            }
        }
        out
    }

    #[test]
    fn a_file_reads_its_formats_and_its_records() {
        let bytes = synthetic();
        let mut file = File::open(&bytes).expect("opens");
        let format = file.format("SG2D").expect("SG2D described").clone();
        assert_eq!(format.labels, ["YCOO", "XCOO"]);
        let record = file.next_record().expect("reads").expect("a record");
        let rows = format.read(record.field("SG2D").expect("SG2D present"));
        assert_eq!(
            rows,
            vec![
                vec![Sub::Int(500_000), Sub::Int(-1_200_000)],
                vec![Sub::Int(510_000), Sub::Int(-1_190_000)],
            ],
            "a repeating field reads every row, and four-byte values are signed"
        );
        assert!(file.next_record().expect("reads").is_none(), "one record");
    }

    #[test]
    fn formats_are_read_as_the_standard_writes_them() {
        // The real descriptors from a NOAA cell's descriptive record.
        let dspm = field_format(
            "RCNM!RCID!HDAT!VDAT!SDAT!CSCL!DUNI!HUNI!PUNI!COUN!COMF!SOMF!COMT",
            "(b11,b14,3b11,b14,4b11,2b14,A)",
        )
        .expect("DSPM");
        assert_eq!(dspm.labels.len(), 13);
        assert_eq!(
            dspm.controls,
            vec![
                Control::Unsigned(1),
                Control::Unsigned(4),
                Control::Unsigned(1),
                Control::Unsigned(1),
                Control::Unsigned(1),
                Control::Unsigned(4),
                Control::Unsigned(1),
                Control::Unsigned(1),
                Control::Unsigned(1),
                Control::Unsigned(1),
                Control::Unsigned(4),
                Control::Unsigned(4),
                Control::Text(None),
            ]
        );
        assert!(!dspm.repeats);

        // A bit string is whole bytes, and a starred descriptor repeats.
        let fspt = field_format("*NAME!ORNT!USAG!MASK", "(B(40),3b11)").expect("FSPT");
        assert!(fspt.repeats);
        assert_eq!(fspt.controls[0], Control::Bytes(5));

        // Nested groups with their own repeats.
        let nested = field_format("A!B!C!D", "(2(A(2),I(5)))").expect("nested");
        assert_eq!(
            nested.controls,
            vec![
                Control::Text(Some(2)),
                Control::Int(Some(5)),
                Control::Text(Some(2)),
                Control::Int(Some(5)),
            ]
        );
    }

    /// The real DSPM field of US2ATLOC.000, byte for byte: a 1:700,000
    /// overview chart whose coordinates are in ten-millionths of a degree
    /// and whose soundings are in tenths of a metre.
    #[test]
    fn a_real_parameter_field_reads_its_multipliers() {
        let format = field_format(
            "RCNM!RCID!HDAT!VDAT!SDAT!CSCL!DUNI!HUNI!PUNI!COUN!COMF!SOMF!COMT",
            "(b11,b14,3b11,b14,4b11,2b14,A)",
        )
        .expect("DSPM");
        let field = b"\x14\x01\x00\x00\x00\x02\x10\x0c\x60\xae\x0a\x00\x01\x01\x01\x01\x80\x96\x98\x00\x0a\x00\x00\x00Produced by NOAA\x1f\x1e";
        let row = format.read_one(field);
        let at = |label: &str| row[format.index_of(label).expect(label)].clone();
        assert_eq!(at("RCNM"), Sub::Int(20), "a parameter record");
        assert_eq!(at("CSCL"), Sub::Int(700_000));
        assert_eq!(at("COMF"), Sub::Int(10_000_000));
        assert_eq!(at("SOMF"), Sub::Int(10));
        assert_eq!(at("COMT"), Sub::Text("Produced by NOAA".to_owned()));
    }

    #[test]
    fn a_truncated_file_is_refused_rather_than_guessed_at() {
        assert!(File::open(b"short").is_err());
        let mut bytes = synthetic();
        bytes.truncate(bytes.len() - 4);
        let mut file = File::open(&bytes).expect("the descriptive record is whole");
        assert!(file.next_record().is_err(), "the data record is not");
    }
}
