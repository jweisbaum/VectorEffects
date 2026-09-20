//! S-57 object and attribute codes, by number.
//!
//! A cell names its object classes and attributes by number, not by the
//! acronyms the standard reads in, so drawing one means holding the part of
//! the catalogue that is drawn. Anything not listed keeps its number
//! (`OBJL_137`, `ATTR_45`), which is honest about what is known and still
//! groups alike with alike.
//!
//! **Every entry here was read off real cells, not remembered.** A class is
//! listed only where the charts identify it beyond doubt: `DEPARE` is the
//! area class carrying a depth range, `SOUNDG` is the only class whose
//! geometry is three-dimensional, `LAKARE` is the area class whose names are
//! lakes. `examples/enc_report.rs` is the tool that reads them off, and
//! `tests/enc.rs` holds the table to a cell whose own bytes are in the test.
//! Several classes a full chart would draw are deliberately absent, because
//! guessing a code draws the wrong thing in the right place — the worst
//! failure a chart can have.

/// The object class of a code, or `OBJL_<code>` for one not listed.
pub fn object_class(code: u16) -> String {
    CLASSES
        .iter()
        .find(|(number, _)| *number == code)
        .map(|(_, name)| (*name).to_owned())
        .unwrap_or_else(|| format!("OBJL_{code}"))
}

/// The attribute name of a code, or `ATTR_<code>` for one not listed.
pub fn attribute_name(code: u16) -> String {
    ATTRIBUTES
        .iter()
        .find(|(number, _)| *number == code)
        .map(|(_, name)| (*name).to_owned())
        .unwrap_or_else(|| format!("ATTR_{code}"))
}

/// The classes the chart style knows.
///
/// The beacon block (5-9) and the buoy block (14-19) are alphabetical runs
/// whose every member carries `BCNSHP`/`BOYSHP` with `COLOUR`; both ends of
/// each run are in the charts, which fixes the ones between.
const CLASSES: &[(u16, &str)] = &[
    (4, "ACHARE"),
    (5, "BCNCAR"),
    (6, "BCNISD"),
    (7, "BCNLAT"),
    (8, "BCNSAW"),
    (9, "BCNSPP"),
    (11, "BRIDGE"),
    (12, "BUISGL"),
    (13, "BUAARE"),
    (14, "BOYCAR"),
    (15, "BOYINB"),
    (16, "BOYISD"),
    (17, "BOYLAT"),
    (18, "BOYSAW"),
    (19, "BOYSPP"),
    (20, "CBLARE"),
    (21, "CBLOHD"),
    (22, "CBLSUB"),
    (23, "CANALS"),
    (27, "CTNARE"),
    (30, "COALNE"),
    (42, "DEPARE"),
    (43, "DEPCNT"),
    (46, "DRGARE"),
    (51, "FAIRWY"),
    (65, "HULKES"),
    (69, "LAKARE"),
    (71, "LNDARE"),
    (73, "LNDRGN"),
    (74, "LNDMRK"),
    (75, "LIGHTS"),
    (84, "MORFAC"),
    (86, "OBSTRN"),
    (114, "RECTRC"),
    (119, "SEAARE"),
    (121, "SBDARE"),
    (122, "SLCONS"),
    (129, "SOUNDG"),
    (153, "UWTROC"),
    (154, "UNSARE"),
    (159, "WRECKS"),
];

/// The attributes the chart style reads, and the few it shows.
const ATTRIBUTES: &[(u16, &str)] = &[
    (2, "BCNSHP"),
    (4, "BOYSHP"),
    (36, "CATLAM"),
    (66, "CATSPM"),
    (71, "CATWRK"),
    (75, "COLOUR"),
    (87, "DRVAL1"),
    (88, "DRVAL2"),
    (93, "TECSOU"),
    (102, "INFORM"),
    (107, "LITCHR"),
    (113, "NATSUR"),
    (116, "OBJNAM"),
    (125, "CATOBS"),
    (131, "RESTRN"),
    (133, "SCAMIN"),
    (147, "SORDAT"),
    (148, "SORIND"),
    (174, "VALDCO"),
    (179, "VALSOU"),
    (187, "WATLEV"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_the_table_does_not_hold_keeps_its_number() {
        assert_eq!(object_class(42), "DEPARE");
        assert_eq!(object_class(9999), "OBJL_9999");
        assert_eq!(attribute_name(87), "DRVAL1");
        assert_eq!(attribute_name(9999), "ATTR_9999");
    }

    #[test]
    fn no_code_is_listed_twice() {
        for table in [
            CLASSES.iter().map(|(code, _)| *code).collect::<Vec<_>>(),
            ATTRIBUTES.iter().map(|(code, _)| *code).collect(),
        ] {
            let mut sorted = table.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), table.len(), "a code appears twice");
        }
        for names in [
            CLASSES.iter().map(|(_, name)| *name).collect::<Vec<_>>(),
            ATTRIBUTES.iter().map(|(_, name)| *name).collect(),
        ] {
            let mut sorted = names.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), names.len(), "a name appears twice");
        }
    }
}
