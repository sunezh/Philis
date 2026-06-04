//! A real (if compact) SPICE netlist reader/writer.
//!
//! This module turns SPICE deck text into a structured [`SpiceNetlist`] and
//! serializes it back out again. It is deliberately std-only — no `serde`, no
//! `regex`, no external crates — and has zero dependencies on the rest of the
//! crate, so it can be type-checked and unit-tested in isolation.
//!
//! ## Format coverage
//!
//! The lexer faithfully handles the lexical structure of real decks:
//!
//! * **Full-line comments** — a `*` in column 0 (after left-trim) marks the
//!   whole line a comment.
//! * **Inline comments** — everything from a `;` or `$` to end of line is
//!   stripped (HSPICE / ngspice style). A leading `*` still wins as a full
//!   comment even if a `$`/`;` appears later.
//! * **Line continuations** — a logical line is reassembled from a physical
//!   line plus every following physical line whose first non-blank character is
//!   `+` (the `+` is dropped and replaced by a space).
//! * **Blank lines** are skipped.
//!
//! On the reassembled logical lines it recognizes:
//!
//! * **`.subckt NAME node... [p=v ...]` / `.ends`** blocks, including nested
//!   device bodies. Each definition keeps its port list, parameters, and the
//!   [`Device`]s declared inside it.
//! * **Control directives** — `.include`, `.lib`, `.param`, `.option(s)`,
//!   `.model`, `.global`, `.end`, etc. are recognized and skipped gracefully
//!   (collected verbatim so the writer can round-trip them).
//! * **Device cards** keyed by first letter: `M` (MOSFET: `d g s b model ...`),
//!   `R`/`C`/`L` (two-terminal passives), `Q` (BJT: `c b e [substrate] model`),
//!   `D` (diode: `a c model`), `X` (subckt instance), and a generic fallback
//!   for any other leading letter. Each card yields a [`Device`] with its name,
//!   ordered node connections, optional model/reference name, and parsed
//!   `key=value` parameters.
//!
//! ## Known limitations (STUB-adjacent)
//!
//! * Expressions in `key={...}` / `key='...'` are stored as opaque strings; no
//!   arithmetic is evaluated.
//! * `.param` / `.model` bodies are kept verbatim, not interpreted.
//! * `.include` / `.lib` files are *not* read from disk — the directive is
//!   recorded but its target is not pulled in (the caller owns the filesystem).
//! * Case: SPICE is case-insensitive for keywords; node and device *names* are
//!   preserved verbatim, while directive/keyword matching is case-folded.

use std::collections::BTreeMap;
use std::fmt;

/// One device "card": its name, its ordered node connections, an optional model
/// or subckt-reference name, and any `key=value` parameters.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Device {
    /// The full device name including its leading letter, e.g. `M1`, `Rload`.
    pub name: String,
    /// The leading letter, upper-cased, e.g. `M`, `R`, `X`. `'\0'` if `name` is
    /// empty.
    pub kind: char,
    /// Ordered node connections (terminals).
    pub nodes: Vec<String>,
    /// Model name (for `M`/`Q`/`D`) or referenced subckt (for `X`), if any.
    pub model: Option<String>,
    /// Parsed `key=value` parameters, in source order.
    pub params: Vec<(String, String)>,
}

impl Device {
    /// The number of node terminals on this device.
    pub fn arity(&self) -> usize {
        self.nodes.len()
    }

    /// Look up a parameter value by (case-insensitive) key.
    pub fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

/// A subcircuit definition: its name, declared ports, header parameters, and the
/// devices in its body.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Subckt {
    /// The subckt name from `.subckt NAME ...`.
    pub name: String,
    /// Declared port nodes, in order.
    pub ports: Vec<String>,
    /// Header `key=value` parameters declared on the `.subckt` line.
    pub params: Vec<(String, String)>,
    /// Devices declared between `.subckt` and the matching `.ends`.
    pub body: Vec<Device>,
}

/// A parsed netlist.
///
/// Holds the top-level devices, every `.subckt` definition seen, and any control
/// directives encountered (kept verbatim for round-tripping).
#[derive(Debug, Clone, Default)]
pub struct SpiceNetlist {
    /// Devices currently promoted to the top level.
    top: Vec<Device>,
    /// All subckt definitions, keyed by name for lookup.
    defs: BTreeMap<String, Subckt>,
    /// Subckt names in source order (since `defs` is sorted by key).
    subckt_order: Vec<String>,
    /// Control directives (`.param`, `.model`, `.include`, ...) kept verbatim.
    directives: Vec<String>,
}

/// Errors the parser can raise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpiceError {
    /// A `.subckt` directive had no name token.
    MissingSubcktName {
        /// 1-based logical-line number where the error occurred.
        line: usize,
    },
    /// A `.ends` was seen with no open `.subckt`.
    UnexpectedEnds {
        /// 1-based logical-line number where the error occurred.
        line: usize,
    },
    /// End of input reached while a `.subckt` was still open.
    UnterminatedSubckt {
        /// The name of the subckt left open.
        name: String,
    },
    /// A continuation line (`+ ...`) appeared before any logical line.
    DanglingContinuation {
        /// 1-based physical-line number of the stray continuation.
        line: usize,
    },
}

impl fmt::Display for SpiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpiceError::MissingSubcktName { line } => {
                write!(f, "line {line}: `.subckt` directive is missing a name")
            }
            SpiceError::UnexpectedEnds { line } => {
                write!(f, "line {line}: `.ends` with no matching `.subckt`")
            }
            SpiceError::UnterminatedSubckt { name } => {
                write!(f, "subckt `{name}` is never closed with `.ends`")
            }
            SpiceError::DanglingContinuation { line } => {
                write!(f, "line {line}: continuation `+` with no preceding line")
            }
        }
    }
}

impl std::error::Error for SpiceError {}

impl SpiceNetlist {
    // ---- construction -----------------------------------------------------

    /// Parse SPICE netlist text. Real parser (continuations, comments, subckts,
    /// device cards). See the module docs for the exact coverage.
    pub fn parse(text: &str) -> Result<SpiceNetlist, SpiceError> {
        let logical = reassemble(text)?;

        let mut nl = SpiceNetlist::default();
        // `Some(name)` while we are inside a `.subckt` body.
        let mut open: Option<String> = None;

        for (lineno, line) in logical {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Directive lines begin with `.`.
            if line.starts_with('.') {
                let lower = line.to_ascii_lowercase();
                if lower.starts_with(".subckt") {
                    // Re-slice the original (case-preserving) text after the
                    // keyword to keep node/param names verbatim.
                    let raw_rest = &line[".subckt".len()..];
                    let sub = parse_subckt_header(raw_rest, lineno)?;
                    open = Some(sub.name.clone());
                    if !nl.defs.contains_key(&sub.name) {
                        nl.subckt_order.push(sub.name.clone());
                    }
                    nl.defs.insert(sub.name.clone(), sub);
                    continue;
                }
                if lower.starts_with(".ends") || lower.starts_with(".eom") {
                    if open.take().is_none() {
                        return Err(SpiceError::UnexpectedEnds { line: lineno });
                    }
                    continue;
                }
                if lower.starts_with(".end") {
                    // `.end` terminates the deck; ignore the rest.
                    break;
                }
                // Any other directive: record verbatim, skip gracefully.
                nl.directives.push(line.to_string());
                continue;
            }

            // Otherwise it is a device card.
            let dev = parse_device(line);
            match &open {
                Some(name) => {
                    // Safe: we inserted the def when we opened it.
                    if let Some(def) = nl.defs.get_mut(name) {
                        def.body.push(dev);
                    }
                }
                None => nl.top.push(dev),
            }
        }

        if let Some(name) = open {
            return Err(SpiceError::UnterminatedSubckt { name });
        }
        Ok(nl)
    }

    /// Build a flat netlist directly from a list of device names (used by the
    /// programmatic builder).
    ///
    /// Each string becomes a top-level [`Device`] whose `name` is the string and
    /// whose leading letter is taken as its `kind`; no nodes or params are
    /// attached.
    pub fn from_devices(devices: Vec<String>) -> SpiceNetlist {
        let top = devices
            .into_iter()
            .map(|name| {
                let kind = name
                    .chars()
                    .next()
                    .map(|c| c.to_ascii_uppercase())
                    .unwrap_or('\0');
                Device {
                    name,
                    kind,
                    ..Device::default()
                }
            })
            .collect();
        SpiceNetlist {
            top,
            ..SpiceNetlist::default()
        }
    }

    // ---- queries ----------------------------------------------------------

    /// Total number of devices parsed: top-level devices plus every device
    /// inside every `.subckt` body.
    pub fn device_count(&self) -> usize {
        self.top.len() + self.defs.values().map(|d| d.body.len()).sum::<usize>()
    }

    /// Number of devices currently promoted to the top level.
    pub fn top_device_count(&self) -> usize {
        self.top.len()
    }

    /// Names of all subcircuits defined (`.subckt NAME`), in source order.
    pub fn subckts(&self) -> &[String] {
        &self.subckt_order
    }

    /// Whether the top level currently has no devices.
    pub fn top_is_empty(&self) -> bool {
        self.top.is_empty()
    }

    /// Borrow the parsed top-level devices.
    pub fn top_devices(&self) -> &[Device] {
        &self.top
    }

    /// Look up a subckt definition by name.
    pub fn subckt(&self, name: &str) -> Option<&Subckt> {
        self.defs.get(name)
    }

    /// Borrow the recorded control directives (verbatim).
    pub fn directives(&self) -> &[String] {
        &self.directives
    }

    // ---- promotion --------------------------------------------------------

    /// Promote the named subckt's body to the top level. Returns `true` if the
    /// name existed.
    ///
    /// The subckt's devices are appended to the top level; the definition itself
    /// is retained so the writer can still emit it.
    pub fn select_subckt(&mut self, name: &str) -> bool {
        match self.defs.get(name) {
            Some(def) => {
                let body = def.body.clone();
                self.top.extend(body);
                true
            }
            None => false,
        }
    }

    /// If the top level is empty and at least one subckt exists, promote the
    /// first one (in source order). Infallible; a no-op when the top level is
    /// non-empty or no subckts exist.
    pub fn promote_first_subckt(&mut self) {
        if self.top_is_empty() {
            if let Some(first) = self.subckt_order.first().cloned() {
                self.select_subckt(&first);
            }
        }
    }

    // ---- serialization ----------------------------------------------------

    /// Serialize back to SPICE text (writer).
    ///
    /// Re-parsing the output yields the same [`device_count`](Self::device_count)
    /// and the same set of subckt names.
    pub fn to_spice(&self) -> String {
        let mut out = String::new();
        out.push_str("* Philis SPICE netlist (generated)\n");

        // Control directives first, verbatim.
        for d in &self.directives {
            out.push_str(d);
            out.push('\n');
        }

        // Subckt definitions, in source order.
        for name in &self.subckt_order {
            if let Some(def) = self.defs.get(name) {
                write_subckt(&mut out, def);
            }
        }

        // Top-level devices.
        for dev in &self.top {
            write_device(&mut out, dev);
        }

        out.push_str(".end\n");
        out
    }
}

// ---------------------------------------------------------------------------
// Lexing
// ---------------------------------------------------------------------------

/// Strip an inline comment (`;` or `$`) from a single physical line and trim the
/// trailing whitespace. A leading `*` (after left-trim) makes the entire line a
/// comment, returning an empty string.
fn strip_comment(raw: &str) -> String {
    let trimmed_left = raw.trim_start();
    if trimmed_left.starts_with('*') {
        return String::new();
    }
    // Cut at the first `;` or `$`.
    let cut = raw
        .find(|c| c == ';' || c == '$')
        .map(|i| &raw[..i])
        .unwrap_or(raw);
    cut.trim_end().to_string()
}

/// Reassemble physical lines into logical lines, applying comment stripping and
/// `+` continuations. Returns `(line_number, text)` pairs where `line_number` is
/// the 1-based physical line where the logical line began.
fn reassemble(text: &str) -> Result<Vec<(usize, String)>, SpiceError> {
    let mut out: Vec<(usize, String)> = Vec::new();

    for (idx, raw) in text.lines().enumerate() {
        let lineno = idx + 1;
        let cleaned = strip_comment(raw);
        let trimmed = cleaned.trim_start();

        if let Some(cont) = trimmed.strip_prefix('+') {
            // Continuation: append to the previous logical line.
            match out.last_mut() {
                Some((_, prev)) => {
                    prev.push(' ');
                    prev.push_str(cont.trim());
                }
                None => return Err(SpiceError::DanglingContinuation { line: lineno }),
            }
            continue;
        }

        // Lines that are empty after comment stripping start no logical line.
        // SPICE treats blanks as separators; real decks never place a blank
        // between a card and its continuations.
        if cleaned.trim().is_empty() {
            continue;
        }

        out.push((lineno, cleaned));
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// Split a logical line into whitespace-separated tokens.
fn tokens(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}

/// Parse a `key=value` token. Returns `None` if there is no non-empty key.
fn parse_kv(tok: &str) -> Option<(String, String)> {
    let eq = tok.find('=')?;
    let key = tok[..eq].trim();
    let val = tok[eq + 1..].trim();
    if key.is_empty() {
        return None;
    }
    Some((key.to_string(), val.to_string()))
}

/// Whether a token is a `key=value` parameter.
fn is_kv(tok: &str) -> bool {
    match tok.find('=') {
        Some(i) => !tok[..i].trim().is_empty(),
        None => false,
    }
}

/// Parse the text following the `.subckt` keyword: `NAME port... [k=v ...]`.
fn parse_subckt_header(rest: &str, lineno: usize) -> Result<Subckt, SpiceError> {
    let toks = tokens(rest);
    let mut it = toks.into_iter();
    let name = it
        .next()
        .ok_or(SpiceError::MissingSubcktName { line: lineno })?
        .to_string();

    let mut ports = Vec::new();
    let mut params = Vec::new();
    for tok in it {
        // The optional `params:` separator (HSPICE) is dropped.
        if tok.eq_ignore_ascii_case("params:") {
            continue;
        }
        if let Some(kv) = parse_kv(tok) {
            params.push(kv);
        } else {
            ports.push(tok.to_string());
        }
    }

    Ok(Subckt {
        name,
        ports,
        params,
        body: Vec::new(),
    })
}

/// Parse a device card into a [`Device`].
///
/// The first token is the device name; its leading letter (upper-cased) is the
/// `kind`. Remaining tokens are classified per the device family: a card-letter
/// table fixes how many leading tokens are *nodes* and whether a trailing
/// model/reference name is expected; everything else is split into `key=value`
/// params (anything with `=`) or extra positional nodes (anything without).
fn parse_device(line: &str) -> Device {
    let toks = tokens(line);
    let mut dev = Device::default();
    if toks.is_empty() {
        return dev;
    }

    dev.name = toks[0].to_string();
    dev.kind = dev
        .name
        .chars()
        .next()
        .map(|c| c.to_ascii_uppercase())
        .unwrap_or('\0');

    // Expected fixed node count per family and whether a model name follows the
    // nodes. `None` means "consume positionals until the first `key=value`"; for
    // `X`, the last positional before any `key=value` is the subckt reference.
    let (fixed_nodes, has_model): (Option<usize>, bool) = match dev.kind {
        'M' => (Some(4), true),              // d g s b model
        'Q' => (Some(3), true),              // c b e [substrate] model
        'D' => (Some(2), true),              // anode cathode model
        'J' => (Some(3), true),              // jfet: d g s model
        'R' | 'C' | 'L' => (Some(2), false), // two-terminal passive
        'X' => (None, true),                 // subckt instance: nodes... SUBNAME
        'V' | 'I' => (Some(2), false),       // source
        _ => (None, false),                  // generic
    };

    let rest = &toks[1..];

    match (fixed_nodes, has_model) {
        (Some(n), with_model) => {
            // Take up to `n` leading non-`key=value` tokens as nodes.
            let mut i = 0;
            while i < rest.len() && i < n && !is_kv(rest[i]) {
                dev.nodes.push(rest[i].to_string());
                i += 1;
            }
            // BJT optional substrate node: swallow a 4th non-kv token only if a
            // further non-kv token remains to serve as the model name.
            if dev.kind == 'Q'
                && i < rest.len()
                && !is_kv(rest[i])
                && i + 1 < rest.len()
                && !is_kv(rest[i + 1])
            {
                dev.nodes.push(rest[i].to_string());
                i += 1;
            }
            if with_model && i < rest.len() && !is_kv(rest[i]) {
                dev.model = Some(rest[i].to_string());
                i += 1;
            }
            // Remaining tokens: params, or stray positionals kept as nodes.
            for tok in &rest[i..] {
                if let Some(kv) = parse_kv(tok) {
                    dev.params.push(kv);
                } else {
                    dev.nodes.push((*tok).to_string());
                }
            }
        }
        (None, with_model) => {
            // X / generic: nodes are leading non-kv tokens; for X the last
            // non-kv token is the subckt reference name.
            let kv_start = rest.iter().position(|t| is_kv(t)).unwrap_or(rest.len());
            let (positional, tail) = rest.split_at(kv_start);

            if with_model && !positional.is_empty() {
                // Last positional is the subckt/model name.
                let (nodes, model) = positional.split_at(positional.len() - 1);
                for n in nodes {
                    dev.nodes.push((*n).to_string());
                }
                dev.model = Some(model[0].to_string());
            } else {
                for n in positional {
                    dev.nodes.push((*n).to_string());
                }
            }

            for tok in tail {
                if let Some(kv) = parse_kv(tok) {
                    dev.params.push(kv);
                } else {
                    dev.nodes.push((*tok).to_string());
                }
            }
        }
    }

    dev
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Append a single device card to `out`.
fn write_device(out: &mut String, dev: &Device) {
    out.push_str(&dev.name);
    for n in &dev.nodes {
        out.push(' ');
        out.push_str(n);
    }
    if let Some(m) = &dev.model {
        out.push(' ');
        out.push_str(m);
    }
    for (k, v) in &dev.params {
        out.push(' ');
        out.push_str(k);
        out.push('=');
        out.push_str(v);
    }
    out.push('\n');
}

/// Append a full `.subckt` ... `.ends` block to `out`.
fn write_subckt(out: &mut String, def: &Subckt) {
    out.push_str(".subckt ");
    out.push_str(&def.name);
    for p in &def.ports {
        out.push(' ');
        out.push_str(p);
    }
    for (k, v) in &def.params {
        out.push(' ');
        out.push_str(k);
        out.push('=');
        out.push_str(v);
    }
    out.push('\n');
    for dev in &def.body {
        write_device(out, dev);
    }
    out.push_str(".ends ");
    out.push_str(&def.name);
    out.push('\n');
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_blanks_are_skipped() {
        let src = "\
* this is a full-line comment
   * indented comment still counts

R1 a b 1k ; trailing inline comment
C1 b 0 1p $ dollar inline comment
";
        let nl = SpiceNetlist::parse(src).unwrap();
        assert_eq!(nl.device_count(), 2);
        assert_eq!(nl.top_device_count(), 2);
        assert!(!nl.top_is_empty());
    }

    #[test]
    fn empty_or_directive_only_is_empty() {
        let src = "\
* header only
.param vdd=1.8
.option temp=27
.global vdd gnd
";
        let nl = SpiceNetlist::parse(src).unwrap();
        assert_eq!(nl.device_count(), 0);
        assert!(nl.top_is_empty());
        assert_eq!(nl.directives().len(), 3);
    }

    #[test]
    fn line_continuation_joins() {
        let src = "\
M1 d g s b nmos
+ w=2u l=0.18u
+ nf=4
";
        let nl = SpiceNetlist::parse(src).unwrap();
        assert_eq!(nl.device_count(), 1);
        let m = &nl.top_devices()[0];
        assert_eq!(m.name, "M1");
        assert_eq!(m.kind, 'M');
        assert_eq!(m.nodes, vec!["d", "g", "s", "b"]);
        assert_eq!(m.model.as_deref(), Some("nmos"));
        assert_eq!(m.param("w"), Some("2u"));
        assert_eq!(m.param("l"), Some("0.18u"));
        assert_eq!(m.param("nf"), Some("4"));
    }

    #[test]
    fn dangling_continuation_errors() {
        let err = SpiceNetlist::parse("+ w=1u\n").unwrap_err();
        assert_eq!(err, SpiceError::DanglingContinuation { line: 1 });
    }

    #[test]
    fn device_cards_mrc_params() {
        let src = "\
M1 nd ng ns nb pmos w=4u l=180n
R1 n1 n2 10k
C1 n2 0 1.5p
";
        let nl = SpiceNetlist::parse(src).unwrap();
        assert_eq!(nl.device_count(), 3);

        let m = &nl.top_devices()[0];
        assert_eq!(m.kind, 'M');
        assert_eq!(m.nodes, vec!["nd", "ng", "ns", "nb"]);
        assert_eq!(m.model.as_deref(), Some("pmos"));
        assert_eq!(m.param("w"), Some("4u"));

        let r = &nl.top_devices()[1];
        assert_eq!(r.kind, 'R');
        assert_eq!(r.nodes[0], "n1");
        assert_eq!(r.nodes[1], "n2");
        assert_eq!(r.model, None);

        let c = &nl.top_devices()[2];
        assert_eq!(c.kind, 'C');
        assert_eq!(c.nodes[0], "n2");
        assert_eq!(c.nodes[1], "0");
    }

    #[test]
    fn subckt_block_and_count() {
        let src = "\
.subckt inv in out vdd vss
M1 out in vdd vdd pmos w=2u l=180n
M2 out in vss vss nmos w=1u l=180n
.ends inv
X1 a y vdd vss inv
";
        let nl = SpiceNetlist::parse(src).unwrap();
        // 2 inside the subckt + 1 top-level X instance.
        assert_eq!(nl.device_count(), 3);
        assert_eq!(nl.top_device_count(), 1);
        assert_eq!(nl.subckts(), &["inv".to_string()]);

        let def = nl.subckt("inv").unwrap();
        assert_eq!(def.ports, vec!["in", "out", "vdd", "vss"]);
        assert_eq!(def.body.len(), 2);

        let x = &nl.top_devices()[0];
        assert_eq!(x.kind, 'X');
        assert_eq!(x.name, "X1");
        assert_eq!(x.nodes, vec!["a", "y", "vdd", "vss"]);
        assert_eq!(x.model.as_deref(), Some("inv"));
    }

    #[test]
    fn unexpected_ends_errors() {
        let err = SpiceNetlist::parse(".ends\n").unwrap_err();
        assert_eq!(err, SpiceError::UnexpectedEnds { line: 1 });
    }

    #[test]
    fn unterminated_subckt_errors() {
        let err = SpiceNetlist::parse(".subckt foo a b\nR1 a b 1k\n").unwrap_err();
        assert_eq!(
            err,
            SpiceError::UnterminatedSubckt {
                name: "foo".to_string()
            }
        );
    }

    #[test]
    fn missing_subckt_name_errors() {
        let err = SpiceNetlist::parse(".subckt\n").unwrap_err();
        assert_eq!(err, SpiceError::MissingSubcktName { line: 1 });
    }

    #[test]
    fn select_subckt_promotes_body() {
        let src = "\
.subckt inv in out
M1 out in 0 0 nmos
M2 out in 0 0 pmos
.ends
";
        let mut nl = SpiceNetlist::parse(src).unwrap();
        assert!(nl.top_is_empty());
        assert!(!nl.select_subckt("nope"));
        assert!(nl.select_subckt("inv"));
        assert!(!nl.top_is_empty());
        assert_eq!(nl.top_device_count(), 2);
    }

    #[test]
    fn promote_first_subckt_when_empty() {
        let src = "\
.subckt a x y
R1 x y 1k
.ends
.subckt b p q
C1 p q 1p
.ends
";
        let mut nl = SpiceNetlist::parse(src).unwrap();
        assert!(nl.top_is_empty());
        nl.promote_first_subckt();
        // First in source order is `a` (one device).
        assert_eq!(nl.top_device_count(), 1);
        assert_eq!(nl.top_devices()[0].name, "R1");
    }

    #[test]
    fn promote_first_subckt_noop_when_nonempty() {
        let src = "\
R1 a b 1k
.subckt s x y
C1 x y 1p
.ends
";
        let mut nl = SpiceNetlist::parse(src).unwrap();
        assert!(!nl.top_is_empty());
        let before = nl.top_device_count();
        nl.promote_first_subckt();
        assert_eq!(nl.top_device_count(), before);
    }

    #[test]
    fn from_devices_is_flat() {
        let nl = SpiceNetlist::from_devices(vec!["M1".into(), "R1".into(), "C1".into()]);
        assert_eq!(nl.device_count(), 3);
        assert_eq!(nl.top_device_count(), 3);
        assert!(!nl.top_is_empty());
        assert_eq!(nl.top_devices()[0].kind, 'M');
    }

    #[test]
    fn from_devices_empty_is_empty() {
        let nl = SpiceNetlist::from_devices(vec![]);
        assert_eq!(nl.device_count(), 0);
        assert!(nl.top_is_empty());
    }

    #[test]
    fn to_spice_round_trip_preserves_count() {
        let src = "\
* a small deck
.param vdd=1.8
.subckt inv in out vdd vss
M1 out in vdd vdd pmos w=2u l=180n
M2 out in vss vss nmos w=1u l=180n
.ends inv
X1 a y vdd vss inv
R1 y 0 10k
";
        let nl = SpiceNetlist::parse(src).unwrap();
        let text = nl.to_spice();
        let again = SpiceNetlist::parse(&text).unwrap();

        assert_eq!(again.device_count(), nl.device_count());
        assert_eq!(again.subckts(), nl.subckts());
        assert_eq!(again.top_device_count(), nl.top_device_count());
    }

    #[test]
    fn round_trip_after_promotion() {
        let src = "\
.subckt inv in out
M1 out in 0 0 nmos w=1u
.ends
";
        let mut nl = SpiceNetlist::parse(src).unwrap();
        nl.promote_first_subckt();
        let text = nl.to_spice();
        let again = SpiceNetlist::parse(&text).unwrap();
        // Top-level promoted M1 + the still-emitted subckt body M1 => 2.
        assert_eq!(again.device_count(), nl.device_count());
    }
}
