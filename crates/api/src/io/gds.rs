//! GDSII (Calma GDS Stream) layout reader and writer.
//!
//! # Format overview
//!
//! A GDSII stream is a flat sequence of **records**. Every record is laid out as:
//!
//! ```text
//! +-----------------+--------+--------+----------------------+
//! | length (u16 BE) |  rtype |  dtype |     data ...         |
//! +-----------------+--------+--------+----------------------+
//!   2 bytes           1 byte   1 byte   length-4 bytes
//! ```
//!
//! * `length` is the *total* record length in bytes, including the 4-byte header.
//! * `rtype` is the record type (e.g. `BOUNDARY`, `XY`).
//! * `dtype` is the data type (no-data, INT2, INT4, REAL8, ASCII).
//!
//! The logical document structure handled here is:
//!
//! ```text
//! HEADER BGNLIB LIBNAME UNITS
//!   ( BGNSTR STRNAME
//!       ( BOUNDARY | PATH | SREF ... ENDEL )*
//!     ENDSTR )*
//! ENDLIB
//! ```
//!
//! ## Data encodings
//!
//! * Integers are stored big-endian (INT2 = `i16`, INT4 = `i32`).
//! * ASCII strings are NUL-padded to an even length.
//! * `UNITS` stores two 8-byte **GDSII reals** (excess-64, base-16 floating
//!   point): the size of a database unit in user units, and in meters.
//!
//! ## Scope / limitations (STUB)
//!
//! This module implements exactly what the Philis place & route flow needs:
//! library/cell structure, `UNITS`, and rectangular `BOUNDARY` elements. It is a
//! tolerant reader — it *counts* `PATH`/`SREF` elements (so `element_count`
//! round-trips for them) but only stores geometry for `BOUNDARY`. Records it does
//! not understand (ELFLAGS, PLEX, PROPATTR, WIDTH, etc.) are skipped on read.
//! Reference elements (`SREF`/`AREF`) are not fully synthesized on write. A full
//! GDSII implementation is tracked in `STUBS.md`.

use std::collections::BTreeMap;
use std::fmt;

// ---------------------------------------------------------------------------
// Record type codes (the 1-byte record-type field).
// ---------------------------------------------------------------------------

const RT_HEADER: u8 = 0x00;
const RT_BGNLIB: u8 = 0x01;
const RT_LIBNAME: u8 = 0x02;
const RT_UNITS: u8 = 0x03;
const RT_ENDLIB: u8 = 0x04;
const RT_BGNSTR: u8 = 0x05;
const RT_STRNAME: u8 = 0x06;
const RT_ENDSTR: u8 = 0x07;
const RT_BOUNDARY: u8 = 0x08;
const RT_PATH: u8 = 0x09;
const RT_SREF: u8 = 0x0a;
const RT_LAYER: u8 = 0x0d;
const RT_DATATYPE: u8 = 0x0e;
const RT_XY: u8 = 0x10;
const RT_ENDEL: u8 = 0x11;

// ---------------------------------------------------------------------------
// Data type codes (the 1-byte data-type field).
// ---------------------------------------------------------------------------

const DT_NODATA: u8 = 0x00;
const DT_INT2: u8 = 0x02;
const DT_INT4: u8 = 0x03;
const DT_REAL8: u8 = 0x05;
const DT_ASCII: u8 = 0x06;

// ===========================================================================
// Errors
// ===========================================================================

/// Errors produced while parsing a GDSII stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GdsError {
    /// The stream ended in the middle of a record header.
    UnexpectedEof,
    /// A record declared a length smaller than its 4-byte header, or one that
    /// overruns the remaining input (e.g. a truncated stream).
    BadRecordLength {
        /// The byte offset where the bad record begins.
        offset: usize,
        /// The (invalid) length the record declared.
        len: usize,
    },
    /// A record carried a payload whose size is invalid for its data type
    /// (e.g. an INT2 record whose payload is not a multiple of 2 bytes).
    BadPayload {
        /// The record type byte.
        rtype: u8,
        /// The payload length that was rejected.
        len: usize,
    },
    /// The stream did not begin with a `HEADER` record.
    MissingHeader,
}

impl fmt::Display for GdsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GdsError::UnexpectedEof => write!(f, "unexpected end of GDSII stream"),
            GdsError::BadRecordLength { offset, len } => {
                write!(f, "bad record length {len} at offset {offset}")
            }
            GdsError::BadPayload { rtype, len } => {
                write!(f, "bad payload length {len} for record type 0x{rtype:02x}")
            }
            GdsError::MissingHeader => write!(f, "stream does not start with a HEADER record"),
        }
    }
}

impl std::error::Error for GdsError {}

// ===========================================================================
// Data model
// ===========================================================================

/// One element within a cell. Only `Boundary` carries geometry; `Path` and
/// `Sref` are tracked so element counts survive a read/write round-trip.
#[derive(Debug, Clone)]
enum Element {
    /// A filled polygon. `xy` is a flat list of database-unit coordinates
    /// `[x0, y0, x1, y1, ...]`, closed (last point equals first).
    Boundary {
        layer: i16,
        datatype: i16,
        xy: Vec<i32>,
    },
    /// A path element (geometry not retained by this stub).
    Path,
    /// A structure reference (target not retained by this stub).
    Sref,
}

/// A structure (cell): a named collection of elements.
#[derive(Debug, Clone)]
struct Cell {
    name: String,
    elements: Vec<Element>,
}

/// A parsed or constructed GDSII library.
#[derive(Debug, Clone)]
pub struct Gds {
    lib_name: String,
    /// Size of one database unit expressed in user units (`UNITS[0]`).
    user_unit: f64,
    /// Size of one database unit expressed in meters (`UNITS[1]`).
    db_unit_meters: f64,
    /// Cells in insertion order.
    cells: Vec<Cell>,
    /// Maps cell name -> index in `cells` for fast lookup.
    cell_index: BTreeMap<String, usize>,
}

impl Gds {
    /// A new, empty library with the given name.
    ///
    /// Defaults to a 1-micron user unit and a 1-nanometre database unit
    /// (`user_unit = 1e-3`, `db_unit_meters = 1e-9`), the conventional choice
    /// for a layout whose coordinates are integer nanometres.
    pub fn new(lib_name: &str) -> Gds {
        Gds {
            lib_name: lib_name.to_string(),
            user_unit: 1e-3,
            db_unit_meters: 1e-9,
            cells: Vec::new(),
            cell_index: BTreeMap::new(),
        }
    }

    /// The library name.
    pub fn lib_name(&self) -> &str {
        &self.lib_name
    }

    /// Number of structures/cells.
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// Total number of elements (boundaries, paths, references) across all cells.
    pub fn element_count(&self) -> usize {
        self.cells.iter().map(|c| c.elements.len()).sum()
    }

    /// Add a structure/cell with the given name; returns whether it was newly
    /// created (`false` if a cell of that name already existed).
    pub fn add_cell(&mut self, name: &str) -> bool {
        if self.cell_index.contains_key(name) {
            return false;
        }
        let idx = self.cells.len();
        self.cells.push(Cell {
            name: name.to_string(),
            elements: Vec::new(),
        });
        self.cell_index.insert(name.to_string(), idx);
        true
    }

    /// Add a rectangular `BOUNDARY` (in database units) on the given layer to the
    /// named cell, creating the cell if absent. `(x, y)` is the lower-left
    /// corner; `w`/`h` are width and height. The rectangle is emitted as the
    /// usual 5 closed points.
    pub fn add_rect(&mut self, cell: &str, layer: i16, x: i32, y: i32, w: i32, h: i32) {
        let idx = match self.cell_index.get(cell) {
            Some(&i) => i,
            None => {
                self.add_cell(cell);
                self.cells.len() - 1
            }
        };
        // Closed rectangle: lower-left, lower-right, upper-right, upper-left, back.
        let xy = vec![x, y, x + w, y, x + w, y + h, x, y + h, x, y];
        self.cells[idx].elements.push(Element::Boundary {
            layer,
            datatype: 0,
            xy,
        });
    }

    // -------------------------------------------------------------------
    // Writing
    // -------------------------------------------------------------------

    /// Serialize to a GDSII binary stream.
    ///
    /// `Gds::read(&g.write())` round-trips: the result has the same
    /// `lib_name`, `cell_count`, and `element_count`.
    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::new();

        // HEADER: stream version 600 (GDSII 6.0).
        write_int2(&mut out, RT_HEADER, &[600]);
        // BGNLIB: last-modified + last-accessed timestamps (12 INT2, zeroed).
        write_int2(&mut out, RT_BGNLIB, &[0; 12]);
        // LIBNAME.
        write_ascii(&mut out, RT_LIBNAME, &self.lib_name);
        // UNITS: [db-unit-in-user-units, db-unit-in-meters].
        write_real8(&mut out, RT_UNITS, &[self.user_unit, self.db_unit_meters]);

        for cell in &self.cells {
            write_int2(&mut out, RT_BGNSTR, &[0; 12]);
            write_ascii(&mut out, RT_STRNAME, &cell.name);
            for el in &cell.elements {
                match el {
                    Element::Boundary {
                        layer,
                        datatype,
                        xy,
                    } => {
                        write_nodata(&mut out, RT_BOUNDARY);
                        write_int2(&mut out, RT_LAYER, &[*layer]);
                        write_int2(&mut out, RT_DATATYPE, &[*datatype]);
                        write_int4(&mut out, RT_XY, xy);
                        write_nodata(&mut out, RT_ENDEL);
                    }
                    // Paths/refs aren't fully modelled; emit a minimal,
                    // valid-shaped element so the count is preserved.
                    Element::Path => {
                        write_nodata(&mut out, RT_PATH);
                        write_int2(&mut out, RT_LAYER, &[0]);
                        write_int2(&mut out, RT_DATATYPE, &[0]);
                        write_int4(&mut out, RT_XY, &[0, 0]);
                        write_nodata(&mut out, RT_ENDEL);
                    }
                    Element::Sref => {
                        write_nodata(&mut out, RT_SREF);
                        write_ascii(&mut out, RT_STRNAME, "");
                        write_int4(&mut out, RT_XY, &[0, 0]);
                        write_nodata(&mut out, RT_ENDEL);
                    }
                }
            }
            write_nodata(&mut out, RT_ENDSTR);
        }

        write_nodata(&mut out, RT_ENDLIB);
        out
    }

    // -------------------------------------------------------------------
    // Reading
    // -------------------------------------------------------------------

    /// Parse a GDSII binary stream.
    pub fn read(bytes: &[u8]) -> Result<Gds, GdsError> {
        let mut r = Reader { buf: bytes, pos: 0 };

        let mut gds = Gds {
            lib_name: String::new(),
            user_unit: 1e-3,
            db_unit_meters: 1e-9,
            cells: Vec::new(),
            cell_index: BTreeMap::new(),
        };

        // The very first record must be HEADER.
        let first = match r.next_record()? {
            Some(rec) => rec,
            None => return Err(GdsError::MissingHeader),
        };
        if first.rtype != RT_HEADER {
            return Err(GdsError::MissingHeader);
        }

        // Parsing state.
        let mut cur_cell: Option<usize> = None;
        // Element being assembled between its start record and its ENDEL.
        let mut pending: Option<PendingElement> = None;

        while let Some(rec) = r.next_record()? {
            match rec.rtype {
                RT_LIBNAME => {
                    gds.lib_name = parse_ascii(rec.data);
                }
                RT_UNITS => {
                    let vals = parse_real8(rec)?;
                    if vals.len() >= 2 {
                        gds.user_unit = vals[0];
                        gds.db_unit_meters = vals[1];
                    }
                }
                RT_BGNSTR => {
                    cur_cell = None; // STRNAME provides the name next
                }
                RT_STRNAME => {
                    // STRNAME inside an SREF names the referenced cell; ignore it
                    // (the stub does not retain reference targets).
                    if !matches!(pending, Some(PendingElement::Sref)) {
                        let name = parse_ascii(rec.data);
                        gds.add_cell(&name);
                        cur_cell = gds.cell_index.get(&name).copied();
                    }
                }
                RT_ENDSTR => {
                    cur_cell = None;
                }
                RT_BOUNDARY => {
                    pending = Some(PendingElement::Boundary {
                        layer: 0,
                        datatype: 0,
                        xy: Vec::new(),
                    });
                }
                RT_PATH => {
                    pending = Some(PendingElement::Path);
                }
                RT_SREF => {
                    pending = Some(PendingElement::Sref);
                }
                RT_LAYER => {
                    let v = parse_int2(rec)?;
                    if let Some(PendingElement::Boundary { layer, .. }) = &mut pending {
                        if let Some(&l) = v.first() {
                            *layer = l;
                        }
                    }
                }
                RT_DATATYPE => {
                    let v = parse_int2(rec)?;
                    if let Some(PendingElement::Boundary { datatype, .. }) = &mut pending {
                        if let Some(&d) = v.first() {
                            *datatype = d;
                        }
                    }
                }
                RT_XY => {
                    let v = parse_int4(rec)?;
                    if let Some(PendingElement::Boundary { xy, .. }) = &mut pending {
                        *xy = v;
                    }
                }
                RT_ENDEL => {
                    if let Some(p) = pending.take() {
                        let el = p.into_element();
                        match cur_cell {
                            Some(idx) => gds.cells[idx].elements.push(el),
                            None => {
                                // Element outside any structure — tolerate by
                                // creating an anonymous cell so the count is kept.
                                let anon = format!("$ORPHAN_{}", gds.cells.len());
                                gds.add_cell(&anon);
                                let idx = gds.cells.len() - 1;
                                gds.cells[idx].elements.push(el);
                            }
                        }
                    }
                }
                RT_ENDLIB => break,
                // HEADER (already consumed), BGNLIB, and any unrecognized
                // records are skipped.
                _ => {}
            }
        }

        Ok(gds)
    }
}

/// An element under construction during parsing.
enum PendingElement {
    Boundary {
        layer: i16,
        datatype: i16,
        xy: Vec<i32>,
    },
    Path,
    Sref,
}

impl PendingElement {
    fn into_element(self) -> Element {
        match self {
            PendingElement::Boundary {
                layer,
                datatype,
                xy,
            } => Element::Boundary {
                layer,
                datatype,
                xy,
            },
            PendingElement::Path => Element::Path,
            PendingElement::Sref => Element::Sref,
        }
    }
}

// ===========================================================================
// Low-level record reader
// ===========================================================================

/// A borrowed view of one record's type, data type, and payload.
struct Record<'a> {
    rtype: u8,
    #[allow(dead_code)]
    dtype: u8,
    data: &'a [u8],
}

/// Cursor over a GDSII byte stream.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Read the next record, or `None` at clean end-of-stream.
    fn next_record(&mut self) -> Result<Option<Record<'a>>, GdsError> {
        let remaining = self.buf.len() - self.pos;
        if remaining == 0 {
            return Ok(None);
        }
        if remaining < 4 {
            return Err(GdsError::UnexpectedEof);
        }
        let offset = self.pos;
        let len = u16::from_be_bytes([self.buf[offset], self.buf[offset + 1]]) as usize;
        // A record must at least contain its 4-byte header...
        if len < 4 {
            return Err(GdsError::BadRecordLength { offset, len });
        }
        // ...and must not overrun the buffer (truncated stream).
        if len > remaining {
            return Err(GdsError::BadRecordLength { offset, len });
        }
        let rtype = self.buf[offset + 2];
        let dtype = self.buf[offset + 3];
        let data = &self.buf[offset + 4..offset + len];
        self.pos += len;
        Ok(Some(Record { rtype, dtype, data }))
    }
}

// ---------------------------------------------------------------------------
// Payload parsers
// ---------------------------------------------------------------------------

fn parse_ascii(data: &[u8]) -> String {
    // Strip the trailing NUL pad (if any) and decode as ASCII/Latin-1.
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    data[..end].iter().map(|&b| b as char).collect()
}

fn parse_int2(rec: Record) -> Result<Vec<i16>, GdsError> {
    let data = rec.data;
    if data.len() % 2 != 0 {
        return Err(GdsError::BadPayload {
            rtype: rec.rtype,
            len: data.len(),
        });
    }
    Ok(data
        .chunks_exact(2)
        .map(|c| i16::from_be_bytes([c[0], c[1]]))
        .collect())
}

fn parse_int4(rec: Record) -> Result<Vec<i32>, GdsError> {
    let data = rec.data;
    if data.len() % 4 != 0 {
        return Err(GdsError::BadPayload {
            rtype: rec.rtype,
            len: data.len(),
        });
    }
    Ok(data
        .chunks_exact(4)
        .map(|c| i32::from_be_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

fn parse_real8(rec: Record) -> Result<Vec<f64>, GdsError> {
    let data = rec.data;
    if data.len() % 8 != 0 {
        return Err(GdsError::BadPayload {
            rtype: rec.rtype,
            len: data.len(),
        });
    }
    Ok(data
        .chunks_exact(8)
        .map(|c| {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(c);
            real8_to_f64(bytes)
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Record writers
// ---------------------------------------------------------------------------

/// Push a record header (`length`, `rtype`, `dtype`) for a payload of
/// `payload_len` bytes.
fn write_header(out: &mut Vec<u8>, rtype: u8, dtype: u8, payload_len: usize) {
    let total = (payload_len + 4) as u16;
    out.extend_from_slice(&total.to_be_bytes());
    out.push(rtype);
    out.push(dtype);
}

fn write_nodata(out: &mut Vec<u8>, rtype: u8) {
    write_header(out, rtype, DT_NODATA, 0);
}

fn write_int2(out: &mut Vec<u8>, rtype: u8, vals: &[i16]) {
    write_header(out, rtype, DT_INT2, vals.len() * 2);
    for &v in vals {
        out.extend_from_slice(&v.to_be_bytes());
    }
}

fn write_int4(out: &mut Vec<u8>, rtype: u8, vals: &[i32]) {
    write_header(out, rtype, DT_INT4, vals.len() * 4);
    for &v in vals {
        out.extend_from_slice(&v.to_be_bytes());
    }
}

fn write_real8(out: &mut Vec<u8>, rtype: u8, vals: &[f64]) {
    write_header(out, rtype, DT_REAL8, vals.len() * 8);
    for &v in vals {
        out.extend_from_slice(&f64_to_real8(v));
    }
}

fn write_ascii(out: &mut Vec<u8>, rtype: u8, s: &str) {
    // ASCII strings are NUL-padded to an even length.
    let bytes = s.as_bytes();
    let padded = bytes.len() + (bytes.len() & 1);
    write_header(out, rtype, DT_ASCII, padded);
    out.extend_from_slice(bytes);
    if bytes.len() & 1 == 1 {
        out.push(0);
    }
}

// ===========================================================================
// GDSII 8-byte REAL (excess-64, base-16 floating point)
// ===========================================================================
//
// Layout, big-endian, bit 0 = MSB of byte 0:
//
//   bit  0      : sign (1 = negative)
//   bits 1..=7  : exponent, excess-64, power of SIXTEEN
//   bits 8..=63 : mantissa, a fraction in [1/16, 1)
//
//   value = (-1)^sign * mantissa * 16^(exponent - 64)
//
// The mantissa's binary point sits just left of bit 8, so bit 8 is 1/2,
// bit 9 is 1/4, etc. To stay normalized the top hex digit (bits 8..=11) is
// nonzero, i.e. mantissa >= 1/16.

/// Decode an 8-byte GDSII real into an `f64`.
fn real8_to_f64(bytes: [u8; 8]) -> f64 {
    let negative = (bytes[0] & 0x80) != 0;
    let exponent = (bytes[0] & 0x7f) as i32 - 64;

    // 56-bit mantissa as an unsigned integer (bits 8..=63).
    let mut mantissa: u64 = 0;
    for &b in &bytes[1..] {
        mantissa = (mantissa << 8) | b as u64;
    }
    if mantissa == 0 {
        return 0.0;
    }

    // Mantissa fraction = mantissa / 2^56; value = fraction * 16^exponent.
    let fraction = mantissa as f64 / ((1u64 << 56) as f64);
    let value = fraction * 16f64.powi(exponent);
    if negative {
        -value
    } else {
        value
    }
}

/// Encode an `f64` as an 8-byte GDSII real.
fn f64_to_real8(value: f64) -> [u8; 8] {
    if value == 0.0 || !value.is_finite() {
        return [0u8; 8];
    }

    let negative = value < 0.0;
    let mut v = value.abs();

    // Find the base-16 exponent so the mantissa fraction lands in [1/16, 1).
    let mut exponent: i32 = 0;
    while v >= 1.0 {
        v /= 16.0;
        exponent += 1;
    }
    while v < 1.0 / 16.0 {
        v *= 16.0;
        exponent -= 1;
    }
    // Now v in [1/16, 1). Mantissa integer = round(v * 2^56).
    let mut mantissa = (v * (1u64 << 56) as f64).round() as u64;

    // Rounding may push the mantissa to 2^56 (== 1.0); renormalize.
    if mantissa >= (1u64 << 56) {
        mantissa >>= 4;
        exponent += 1;
    }

    let biased = exponent + 64;
    // Clamp to the representable excess-64 range [0, 127]; out-of-range values
    // fall back to zero, matching how real implementations treat overflow.
    if !(0..=127).contains(&biased) {
        return [0u8; 8];
    }

    let mut out = [0u8; 8];
    out[0] = (biased as u8) & 0x7f;
    if negative {
        out[0] |= 0x80;
    }
    for i in (1..8).rev() {
        out[i] = (mantissa & 0xff) as u8;
        mantissa >>= 8;
    }
    out
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn real_roundtrip(x: f64) {
        let enc = f64_to_real8(x);
        let dec = real8_to_f64(enc);
        // GDSII reals carry ~14-15 significant decimal digits; allow tiny error.
        let tol = x.abs() * 1e-12 + 1e-300;
        assert!(
            (dec - x).abs() <= tol,
            "real round-trip failed: {x} -> {dec} (enc = {enc:02x?})"
        );
    }

    #[test]
    fn real_encode_decode_units() {
        real_roundtrip(1e-6);
        real_roundtrip(1e-9);
        real_roundtrip(1.0);
        real_roundtrip(0.5);
        real_roundtrip(1e-3);
        real_roundtrip(-2.5);
        real_roundtrip(12345.678);
    }

    #[test]
    fn real_zero_is_all_zero() {
        assert_eq!(f64_to_real8(0.0), [0u8; 8]);
        assert_eq!(real8_to_f64([0u8; 8]), 0.0);
    }

    #[test]
    fn real_one_canonical_encoding() {
        // 1.0 = (1/16) * 16^1  => exponent 1 (biased 0x41), mantissa 0x10....
        let enc = f64_to_real8(1.0);
        assert_eq!(enc[0], 0x41);
        assert_eq!(enc[1], 0x10);
        assert_eq!(&enc[2..], &[0u8; 6]);
    }

    #[test]
    fn library_round_trip() {
        let mut g = Gds::new("PHILIS");
        g.add_rect("TOP", 1, 0, 0, 100, 200);
        g.add_rect("TOP", 2, 50, 50, 10, 10);
        g.add_rect("CELLB", 1, -100, -100, 100, 100);

        assert_eq!(g.lib_name(), "PHILIS");
        assert_eq!(g.cell_count(), 2);
        assert_eq!(g.element_count(), 3);

        let bytes = g.write();
        let g2 = Gds::read(&bytes).expect("read back");

        assert_eq!(g2.lib_name(), "PHILIS");
        assert_eq!(g2.cell_count(), 2);
        assert_eq!(g2.element_count(), 3);

        // Units survive too.
        assert!((g2.user_unit - g.user_unit).abs() < 1e-18);
        assert!((g2.db_unit_meters - g.db_unit_meters).abs() < 1e-18);
    }

    #[test]
    fn add_cell_dedup() {
        let mut g = Gds::new("LIB");
        assert!(g.add_cell("A"));
        assert!(!g.add_cell("A"));
        assert_eq!(g.cell_count(), 1);
    }

    #[test]
    fn add_rect_creates_cell_and_closes_polygon() {
        let mut g = Gds::new("LIB");
        g.add_rect("NEW", 5, 1, 2, 3, 4);
        assert_eq!(g.cell_count(), 1);
        assert_eq!(g.element_count(), 1);
        match &g.cells[0].elements[0] {
            Element::Boundary { layer, xy, .. } => {
                assert_eq!(*layer, 5);
                assert_eq!(xy.len(), 10); // 5 closed points
                assert_eq!(&xy[0..2], &[1, 2]); // first point...
                assert_eq!(&xy[8..10], &[1, 2]); // ...equals last
            }
            _ => panic!("expected boundary"),
        }
    }

    #[test]
    fn malformed_truncated_record() {
        // Build a valid stream, then truncate it mid-record.
        let mut g = Gds::new("LIB");
        g.add_rect("C", 1, 0, 0, 10, 10);
        let mut bytes = g.write();
        // Drop the final few bytes so the last record's declared length overruns.
        bytes.truncate(bytes.len() - 3);
        let err = Gds::read(&bytes).unwrap_err();
        assert!(
            matches!(
                err,
                GdsError::BadRecordLength { .. } | GdsError::UnexpectedEof
            ),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn missing_header_rejected() {
        // A first record that is not HEADER.
        let mut bytes = Vec::new();
        write_nodata(&mut bytes, RT_ENDLIB);
        assert_eq!(Gds::read(&bytes).unwrap_err(), GdsError::MissingHeader);
        // An empty stream also has no header.
        assert_eq!(Gds::read(&[]).unwrap_err(), GdsError::MissingHeader);
    }

    #[test]
    fn bad_record_length_too_small() {
        // length field = 2, which is < the 4-byte header minimum.
        let bytes = [0x00, 0x02, RT_HEADER, DT_INT2];
        let err = Gds::read(&bytes).unwrap_err();
        assert!(matches!(err, GdsError::BadRecordLength { .. }));
    }

    #[test]
    fn display_and_error_trait() {
        let e = GdsError::UnexpectedEof;
        let _s: String = e.to_string();
        // Ensure it is usable as a std::error::Error trait object.
        let _boxed: Box<dyn std::error::Error> = Box::new(e);
    }
}
