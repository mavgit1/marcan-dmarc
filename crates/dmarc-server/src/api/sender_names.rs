//! The display-name overrides as text, a list like the others ([`ListText`]):
//!
//! `GET /api/settings/sender-names` — the overrides.
//! `POST /api/settings/sender-names/preview` — a draft → what saving it would
//! add, change and remove, and its line errors; writes nothing.
//! `PUT /api/settings/sender-names` — a draft → the same summary, saved;
//! refused with `errors` (every bad line) when the text does not parse, and
//! nothing is saved.
//!
//! One `target, name` per line, the target an IP, an IP range (CIDR) or an
//! operator domain; the most specific match wins, and an empty name hides the
//! name a less specific one (or the bundled parsedmarc map) would give. Saved
//! overrides apply to the next request.

use super::mapping::rejected_lines;
use crate::error::AppResult;
use crate::extract::Json;
use crate::view::{LineError, ListText};
use crate::AppState;
use axum::extract::State;
use dmarc_identity::{format_overrides, parse_overrides, NameOverride, SenderNames};
use dmarc_store::{list_sender_names, replace_sender_names, StoreError};
use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use ts_rs::TS;

/// One override as a line of the list has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct SenderNameLine {
    /// An IP, an IP range or an operator domain, as stored.
    pub target: String,
    /// `null`: the name it would get is hidden.
    pub name: Option<String>,
}

impl From<&NameOverride> for SenderNameLine {
    fn from(o: &NameOverride) -> Self {
        Self {
            target: o.target.clone(),
            name: o.name.clone(),
        }
    }
}

/// What saving a list changes against the stored one.
#[derive(Debug, Default, Serialize, TS)]
#[ts(export)]
pub struct SenderNameChanges {
    /// Targets not listed before.
    pub add: Vec<SenderNameLine>,
    /// Targets listed before with another name; the new one.
    pub change: Vec<SenderNameLine>,
    /// Targets no longer listed.
    pub remove: Vec<String>,
}

/// What saving a list changes (preview) or changed (save).
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct SenderNamesSummary {
    /// Empty while the text has errors.
    pub changes: SenderNameChanges,
    pub errors: Vec<LineError>,
}

async fn stored_overrides(pool: &SqlitePool) -> Result<Vec<NameOverride>, StoreError> {
    Ok(list_sender_names(pool)
        .await?
        .into_iter()
        .map(|(target, name)| NameOverride { target, name })
        .collect())
}

/// The bundled names with the stored overrides on top (for startup).
pub async fn load_sender_names(pool: &SqlitePool) -> Result<SenderNames, StoreError> {
    Ok(SenderNames::with_overrides(stored_overrides(pool).await?))
}

/// `text` read as overrides, or every line that cannot be read.
fn parse(text: &str) -> Result<Vec<NameOverride>, Vec<LineError>> {
    parse_overrides(text).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| LineError {
                line: usize::try_from(e.line).unwrap_or(usize::MAX),
                message: e.message,
            })
            .collect()
    })
}

/// What replacing `stored` with `new` changes, in `new`'s order.
fn changes(stored: &[NameOverride], new: &[NameOverride]) -> SenderNameChanges {
    let before: HashMap<&str, &Option<String>> = stored
        .iter()
        .map(|o| (o.target.as_str(), &o.name))
        .collect();
    let after: HashMap<&str, &Option<String>> =
        new.iter().map(|o| (o.target.as_str(), &o.name)).collect();
    let mut out = SenderNameChanges::default();
    for o in new {
        match before.get(o.target.as_str()) {
            None => out.add.push(o.into()),
            Some(name) if **name != o.name => out.change.push(o.into()),
            Some(_) => {}
        }
    }
    out.remove = stored
        .iter()
        .filter(|o| !after.contains_key(o.target.as_str()))
        .map(|o| o.target.clone())
        .collect();
    out
}

pub async fn get_sender_names(State(state): State<Arc<AppState>>) -> AppResult<Json<ListText>> {
    let overrides = stored_overrides(&state.pool).await?;
    Ok(Json(ListText {
        text: format_overrides(&overrides),
    }))
}

pub async fn preview_sender_names(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ListText>,
) -> AppResult<Json<SenderNamesSummary>> {
    Ok(Json(match parse(&req.text) {
        Ok(overrides) => SenderNamesSummary {
            changes: changes(&stored_overrides(&state.pool).await?, &overrides),
            errors: Vec::new(),
        },
        Err(errors) => SenderNamesSummary {
            changes: SenderNameChanges::default(),
            errors,
        },
    }))
}

pub async fn save_sender_names(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ListText>,
) -> AppResult<Json<SenderNamesSummary>> {
    let overrides = parse(&req.text).map_err(|errors| rejected_lines(&errors, "list"))?;
    let changes = changes(&stored_overrides(&state.pool).await?, &overrides);
    let rows: Vec<(String, Option<String>)> = overrides
        .iter()
        .map(|o| (o.target.clone(), o.name.clone()))
        .collect();
    replace_sender_names(&state.pool, &rows).await?;
    state.set_names(SenderNames::with_overrides(overrides));
    Ok(Json(SenderNamesSummary {
        changes,
        errors: Vec::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(target: &str, name: Option<&str>) -> NameOverride {
        NameOverride {
            target: target.into(),
            name: name.map(str::to_owned),
        }
    }

    #[test]
    fn changes_name_what_is_added_changed_and_removed() {
        let stored = [o("example.com", Some("Example")), o("192.0.2.1", None)];
        let new = [
            o("example.com", Some("Example Corp")),
            o("192.0.2.0/24", Some("Office")),
        ];
        let c = changes(&stored, &new);
        assert_eq!(c.add, [SenderNameLine::from(&new[1])]);
        assert_eq!(c.change, [SenderNameLine::from(&new[0])]);
        assert_eq!(c.remove, ["192.0.2.1"]);
        let same = changes(&stored, &stored);
        assert!(same.add.is_empty() && same.change.is_empty() && same.remove.is_empty());
    }
}
