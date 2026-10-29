//! Catalog names agents can pass without memorizing wire codes.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use sony_ptp3::catalog::code::control::{
    FORCED_FILE_NUMBER_RESET, PIXEL_MAPPING, POWER_OFF, REC_SETTINGS_RESET, RESET_CREATIVE_LOOK,
    RESET_PICTURE_PROFILE, SELECTED_MEDIA_FORMAT, SENSOR_CLEANING, TIME_CODE_PRESET_RESET,
    USER_BIT_PRESET_RESET,
};
use sony_ptp3::catalog::ControlCode;
use sony_ptp3::catalog::{self, Spec};

use crate::error::CliError;

#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub code: u16,
    pub name: &'static str,
    pub slug: String,
    pub datatype: u16,
}

#[derive(Debug)]
pub enum Resolved<'a> {
    Known(&'a CatalogEntry),
    UnknownCode(u16),
}

impl Resolved<'_> {
    pub fn code(&self) -> u16 {
        match self {
            Self::Known(entry) => entry.code,
            Self::UnknownCode(code) => *code,
        }
    }
}

pub fn properties() -> &'static [CatalogEntry] {
    cached(&PROPERTY_CACHE, catalog::PROPERTIES, |_| 0)
}

pub fn controls() -> &'static [CatalogEntry] {
    cached(&CONTROL_CACHE, catalog::CONTROLS, |spec| spec.datatype)
}

pub fn operations() -> &'static [CatalogEntry] {
    cached(&OPERATION_CACHE, catalog::OPERATIONS, |_| 0)
}

pub fn events() -> &'static [CatalogEntry] {
    cached(&EVENT_CACHE, catalog::EVENTS, |_| 0)
}

pub fn find(entries: &[CatalogEntry], code: u16) -> Option<&CatalogEntry> {
    entries
        .binary_search_by_key(&code, |entry| entry.code)
        .ok()
        .map(|index| &entries[index])
}

/// Resolve a slug, catalog name, or `0x` code. Names that are not in the catalog are usage
/// errors. A well-formed code the catalog does not list is returned so the camera can decide.
pub fn resolve<'a>(
    kind: &str,
    query: &str,
    entries: &'a [CatalogEntry],
) -> Result<Resolved<'a>, CliError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(CliError::usage(
            format!("missing {kind}"),
            format!("Pass --name or --code. Run: cameractl {kind} list"),
        ));
    }
    if query.starts_with("0x") || query.starts_with("0X") {
        let code = hex_u16(query).ok_or_else(|| {
            CliError::usage(
                format!("code {query} is not a 16-bit hex value"),
                "Example: --code 0xd21e",
            )
            .input(query)
        })?;
        if let Some(entry) = find(entries, code) {
            return Ok(Resolved::Known(entry));
        }
        return Ok(Resolved::UnknownCode(code));
    }
    let slug = slugify(query);
    let mut matches = entries.iter().filter(|entry| entry.slug == slug);
    let first = matches.next();
    let second = matches.next();
    match (first, second) {
        (Some(entry), None) => Ok(Resolved::Known(entry)),
        (None, _) => Err(unknown(kind, query, &slug, entries)),
        (Some(_), Some(_)) => Err(CliError::usage(
            format!("{kind} {query} matches more than one catalog name"),
            format!("Pass --code instead. Run: cameractl {kind} list --name-contains {slug}"),
        )
        .input(query)),
    }
}

pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !dash && !slug.is_empty() {
            slug.push('-');
            dash = true;
        }
    }
    if slug.ends_with('-') {
        slug.pop();
    }
    slug
}

pub fn closest<'a>(query: &str, candidates: impl Iterator<Item = &'a str>) -> Vec<String> {
    let query = query.to_ascii_lowercase();
    let mut scored = Vec::new();
    for candidate in candidates {
        let distance = levenshtein(&query, candidate);
        let contains = query.len() >= 3 && candidate.contains(&query);
        if distance <= 3 || contains {
            let rank = if distance <= 3 {
                distance
            } else {
                10 + candidate.len()
            };
            scored.push((rank, candidate.len(), candidate.to_owned()));
        }
    }
    scored.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then(left.1.cmp(&right.1))
            .then(left.2.cmp(&right.2))
    });
    scored
        .into_iter()
        .take(3)
        .map(|(_, _, slug)| slug)
        .collect()
}

pub fn control_is_destructive(code: u16) -> bool {
    [
        SELECTED_MEDIA_FORMAT,
        REC_SETTINGS_RESET,
        PIXEL_MAPPING,
        POWER_OFF,
        TIME_CODE_PRESET_RESET,
        USER_BIT_PRESET_RESET,
        SENSOR_CLEANING,
        RESET_PICTURE_PROFILE,
        RESET_CREATIVE_LOOK,
        FORCED_FILE_NUMBER_RESET,
    ]
    .contains(&ControlCode(code))
}

fn unknown(kind: &str, query: &str, slug: &str, entries: &[CatalogEntry]) -> CliError {
    let suggestions = closest(slug, entries.iter().map(|entry| entry.slug.as_str()));
    let message = if suggestions.is_empty() {
        format!("unknown {kind} {query}")
    } else {
        format!(
            "unknown {kind} {query}. Did you mean {}?",
            suggestions.join(", ")
        )
    };
    CliError::usage(
        message,
        format!("Run: cameractl {kind} list --name-contains {slug}"),
    )
    .input(query)
}

fn hex_u16(raw: &str) -> Option<u16> {
    let hex = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X"))?;
    if hex.is_empty() || hex.len() > 4 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    u16::from_str_radix(hex, 16).ok()
}

fn levenshtein(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    if left.len().abs_diff(right.len()) > 3 {
        return left.len().abs_diff(right.len());
    }
    let mut prev: Vec<usize> = (0..=right.len()).collect();
    let mut next = vec![0; right.len() + 1];
    for (i, ca) in left.iter().enumerate() {
        next[0] = i + 1;
        for (j, cb) in right.iter().enumerate() {
            let cost = usize::from(ca != cb);
            next[j + 1] = (prev[j + 1] + 1).min(next[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut next);
    }
    prev[right.len()]
}

fn cached<T>(
    cell: &'static OnceLock<Vec<CatalogEntry>>,
    rows: &'static [T],
    datatype: impl Fn(&T) -> u16 + 'static,
) -> &'static [CatalogEntry]
where
    T: Named,
{
    cell.get_or_init(move || {
        let raw: Vec<(u16, &'static str, u16)> = rows
            .iter()
            .map(|row| (row.code(), row.name(), datatype(row)))
            .collect();
        build(raw)
    })
    .as_slice()
}

trait Named {
    fn code(&self) -> u16;
    fn name(&self) -> &'static str;
}

impl Named for Spec {
    fn code(&self) -> u16 {
        self.code
    }
    fn name(&self) -> &'static str {
        self.name
    }
}

impl Named for catalog::ControlSpec {
    fn code(&self) -> u16 {
        self.code
    }
    fn name(&self) -> &'static str {
        self.name
    }
}

fn build(rows: Vec<(u16, &'static str, u16)>) -> Vec<CatalogEntry> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let slugs: Vec<String> = rows.iter().map(|(_, name, _)| slugify(name)).collect();
    for slug in &slugs {
        *counts.entry(slug.clone()).or_insert(0) += 1;
    }
    rows.into_iter()
        .zip(slugs)
        .map(|((code, name, datatype), slug)| {
            let slug = if counts[&slug] > 1 {
                format!("{slug}-{code:#06x}")
            } else {
                slug
            };
            CatalogEntry {
                code,
                name,
                slug,
                datatype,
            }
        })
        .collect()
}

static PROPERTY_CACHE: OnceLock<Vec<CatalogEntry>> = OnceLock::new();
static CONTROL_CACHE: OnceLock<Vec<CatalogEntry>> = OnceLock::new();
static OPERATION_CACHE: OnceLock<Vec<CatalogEntry>> = OnceLock::new();
static EVENT_CACHE: OnceLock<Vec<CatalogEntry>> = OnceLock::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_match_the_names_agents_see() {
        assert_eq!(slugify("ISO Sensitivity"), "iso-sensitivity");
        assert_eq!(slugify("Near/Far"), "near-far");
        assert_eq!(slugify("AF Area Position (x, y)"), "af-area-position-x-y");
        assert_eq!(slugify("S1 & S2 Button"), "s1-s2-button");
        assert_eq!(
            slugify("ISO Auto Range Limit (min)"),
            "iso-auto-range-limit-min"
        );
    }

    #[test]
    fn catalog_slugs_are_unique() {
        for entries in [properties(), controls(), operations(), events()] {
            let mut seen = BTreeMap::new();
            for entry in entries {
                assert!(
                    seen.insert(entry.slug.as_str(), entry.name).is_none(),
                    "slug {} is shared",
                    entry.slug
                );
            }
        }
    }

    #[test]
    fn iso_resolves_from_the_slug_and_the_code() {
        let slug = resolve("property", "iso-sensitivity", properties()).unwrap();
        let code = resolve("property", "0xd21e", properties()).unwrap();
        assert_eq!(slug.code(), 0xd21e);
        assert_eq!(code.code(), 0xd21e);
        assert!(matches!(
            resolve("property", "0x0000", properties()).unwrap(),
            Resolved::UnknownCode(0)
        ));
    }

    #[test]
    fn a_typo_names_the_nearest_slug() {
        let error = resolve("property", "iso-sensitivty", properties()).unwrap_err();
        assert_eq!(error.exit, crate::error::EXIT_USAGE);
        assert!(error.message.contains("iso-sensitivity"));
        assert_eq!(error.input.as_deref(), Some("iso-sensitivty"));
    }

    #[test]
    fn media_format_is_destructive_and_zoom_is_not() {
        assert!(control_is_destructive(SELECTED_MEDIA_FORMAT.0));
        assert!(!control_is_destructive(0xd2dd));
    }
}
