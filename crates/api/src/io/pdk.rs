//! The PDK (process design kit) reader.
//!
//! A PDK is supplied as **JSON or TOML** following a small schema (technology
//! name, database unit, layer list). This module reads either format and exposes
//! the parsed [`Pdk`]; loading never mutates global state, so one `Pdk` value can
//! be shared (`&pdk`) across many circuits.
//!
//! STUB: the reader is a tolerant, std-only key scanner — it extracts the handful
//! of schema keys it understands and records a warning for anything it doesn't,
//! rather than pulling in `serde`/`toml`. A full schema validator is tracked in
//! `crates/api/STUBS.md`.

use crate::ApiError;

/// The serialized format of a PDK file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdkFormat {
    Json,
    Toml,
}

/// A loaded process design kit.
#[derive(Debug, Clone, Default)]
pub struct Pdk {
    tech: Option<String>,
    db_unit_nm: Option<i64>,
    layers: Vec<String>,
    format: Option<PdkFormat>,
    warnings: Vec<String>,
}

impl Pdk {
    /// Parse a PDK from JSON text.
    pub fn from_json_str(text: &str) -> Result<Pdk, ApiError> {
        Self::parse(text, PdkFormat::Json)
    }

    /// Parse a PDK from a JSON file.
    pub fn from_json_file(path: impl AsRef<std::path::Path>) -> Result<Pdk, ApiError> {
        let text = read(path.as_ref())?;
        Self::parse(&text, PdkFormat::Json)
    }

    /// Parse a PDK from TOML text.
    pub fn from_toml_str(text: &str) -> Result<Pdk, ApiError> {
        Self::parse(text, PdkFormat::Toml)
    }

    /// Parse a PDK from a TOML file.
    pub fn from_toml_file(path: impl AsRef<std::path::Path>) -> Result<Pdk, ApiError> {
        let text = read(path.as_ref())?;
        Self::parse(&text, PdkFormat::Toml)
    }

    /// Load a PDK file, choosing the format from its extension (`.toml` → TOML,
    /// otherwise JSON). The convenience alias used by most callers.
    pub fn from_path(path: impl AsRef<std::path::Path>) -> Result<Pdk, ApiError> {
        let path = path.as_ref();
        let text = read(path)?;
        let fmt = match path.extension().and_then(|e| e.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("toml") => PdkFormat::Toml,
            _ => PdkFormat::Json,
        };
        Self::parse(&text, fmt)
    }

    /// Read a path from the named env var, then load it (extension-detected).
    pub fn from_env(var: &str) -> Result<Pdk, ApiError> {
        let path = std::env::var(var).map_err(|_| ApiError::MissingPdkPathEnv(var.to_string()))?;
        Self::from_path(path)
    }

    /// The technology name, if the schema declared one.
    pub fn tech(&self) -> Option<&str> {
        self.tech.as_deref()
    }
    /// The database unit in nanometers, if declared.
    pub fn db_unit_nm(&self) -> Option<i64> {
        self.db_unit_nm
    }
    /// The declared layer names.
    pub fn layers(&self) -> &[String] {
        &self.layers
    }
    /// The format this PDK was parsed from.
    pub fn format(&self) -> Option<PdkFormat> {
        self.format
    }
    /// Soft warnings produced while loading.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    // ---- the tolerant stub parser ----------------------------------------

    fn parse(text: &str, format: PdkFormat) -> Result<Pdk, ApiError> {
        if text.trim().is_empty() {
            return Err(ApiError::Pdk("empty PDK schema".to_string()));
        }
        let mut pdk = Pdk {
            format: Some(format),
            ..Default::default()
        };
        match format {
            PdkFormat::Json => {
                if !text.contains('{') {
                    pdk.warnings
                        .push("text does not look like JSON (stub parser)".to_string());
                }
                pdk.tech = scan_json_string(text, "tech");
                pdk.db_unit_nm = scan_json_number(text, "db_unit_nm");
                pdk.layers = scan_json_string_array(text, "layers");
            }
            PdkFormat::Toml => {
                pdk.tech = scan_toml_string(text, "tech");
                pdk.db_unit_nm = scan_toml_number(text, "db_unit_nm");
                pdk.layers = scan_toml_string_array(text, "layers");
            }
        }
        if pdk.tech.is_none() {
            pdk.warnings
                .push("schema declares no `tech` key (stub parser)".to_string());
        }
        Ok(pdk)
    }
}

fn read(path: &std::path::Path) -> Result<String, ApiError> {
    std::fs::read_to_string(path).map_err(|e| ApiError::Io(format!("{}: {e}", path.display())))
}

// --- naive, std-only key scanners (STUB; not real JSON/TOML parsers) -------

fn scan_json_string(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let rest = &text[text.find(&needle)? + needle.len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn scan_json_number(text: &str, key: &str) -> Option<i64> {
    let needle = format!("\"{key}\"");
    let rest = &text[text.find(&needle)? + needle.len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '-')
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn scan_json_string_array(text: &str, key: &str) -> Vec<String> {
    let needle = format!("\"{key}\"");
    let Some(start) = text.find(&needle) else {
        return Vec::new();
    };
    let rest = &text[start + needle.len()..];
    let Some(open) = rest.find('[') else {
        return Vec::new();
    };
    let Some(close) = rest[open..].find(']') else {
        return Vec::new();
    };
    split_quoted(&rest[open + 1..open + close])
}

fn scan_toml_string(text: &str, key: &str) -> Option<String> {
    toml_value_line(text, key).and_then(|v| {
        let v = v.trim();
        let v = v.strip_prefix('"')?;
        let end = v.find('"')?;
        Some(v[..end].to_string())
    })
}

fn scan_toml_number(text: &str, key: &str) -> Option<i64> {
    toml_value_line(text, key).and_then(|v| v.trim().parse().ok())
}

fn scan_toml_string_array(text: &str, key: &str) -> Vec<String> {
    match toml_value_line(text, key) {
        Some(v) => {
            let v = v.trim();
            let inner = v
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .unwrap_or("");
            split_quoted(inner)
        }
        None => Vec::new(),
    }
}

/// Return the RHS of a `key = value` TOML line.
fn toml_value_line<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let rest = line.strip_prefix(key)?;
        let rest = rest.trim_start().strip_prefix('=')?;
        Some(rest.trim())
    })
}

/// Split a comma list of `"quoted"` strings, ignoring whitespace.
fn split_quoted(s: &str) -> Vec<String> {
    s.split(',')
        .filter_map(|part| {
            let part = part.trim();
            let part = part.strip_prefix('"')?;
            let end = part.find('"')?;
            Some(part[..end].to_string())
        })
        .collect()
}
