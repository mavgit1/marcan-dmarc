//! The custom DKIM selectors the checker probes for every domain: a list
//! with its own validation, not a settings group, edited like the other
//! lists ([`ListText`]):
//!
//! `GET /api/settings/dkim-selectors` — the selectors, one per line, sorted.
//! `POST /api/settings/dkim-selectors/preview` — a draft → which selectors
//! saving it would add and remove, and its line errors; writes nothing.
//! `PUT /api/settings/dkim-selectors` — a draft → the same summary, saved;
//! refused with `errors` when a selector is not one, and nothing is saved.
//!
//! One selector per line, or several separated by commas, semicolons or
//! spaces; compared and stored in lowercase, repeats dropped.

use crate::api::mapping::rejected_lines;
use crate::error::AppResult;
use crate::extract::Json;
use crate::view::{LineError, ListText};
use crate::AppState;
use axum::extract::State;
use dmarc_dns::dkim::is_valid_selector;
use dmarc_store::{get_custom_dkim_selectors, set_custom_dkim_selectors};
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::Arc;
use ts_rs::TS;

/// What saving a list changes against the stored one.
#[derive(Debug, Default, Serialize, TS)]
#[ts(export)]
pub struct DkimSelectorChanges {
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

/// What saving a list changes (preview) or changed (save).
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct DkimSelectorsSummary {
    /// The selectors that can be read, even while other lines have errors.
    pub changes: DkimSelectorChanges,
    pub errors: Vec<LineError>,
}

/// The selectors of `text`, lowercase, sorted and without repeats, and every
/// line with a token that is not a selector.
fn parse(text: &str) -> (BTreeSet<String>, Vec<LineError>) {
    let mut selectors = BTreeSet::new();
    let mut errors = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let tokens = line
            .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
            .filter(|t| !t.is_empty());
        let mut invalid = Vec::new();
        for token in tokens {
            let selector = token.to_ascii_lowercase();
            if is_valid_selector(&selector) {
                selectors.insert(selector);
            } else {
                invalid.push(token);
            }
        }
        if !invalid.is_empty() {
            errors.push(LineError {
                line: index + 1,
                message: format!("not a DKIM selector: {}", invalid.join(", ")),
            });
        }
    }
    (selectors, errors)
}

/// What replacing `stored` with `new` changes.
fn changes(stored: &[String], new: &BTreeSet<String>) -> DkimSelectorChanges {
    let before: BTreeSet<&String> = stored.iter().collect();
    DkimSelectorChanges {
        add: new
            .iter()
            .filter(|s| !before.contains(s))
            .cloned()
            .collect(),
        remove: stored
            .iter()
            .filter(|s| !new.contains(*s))
            .cloned()
            .collect(),
    }
}

pub async fn get_dkim_selectors(State(state): State<Arc<AppState>>) -> AppResult<Json<ListText>> {
    let selectors = get_custom_dkim_selectors(&state.pool).await?;
    Ok(Json(ListText {
        text: selectors.iter().map(|s| format!("{s}\n")).collect(),
    }))
}

pub async fn preview_dkim_selectors(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ListText>,
) -> AppResult<Json<DkimSelectorsSummary>> {
    let (selectors, errors) = parse(&req.text);
    let stored = get_custom_dkim_selectors(&state.pool).await?;
    Ok(Json(DkimSelectorsSummary {
        changes: changes(&stored, &selectors),
        errors,
    }))
}

pub async fn save_dkim_selectors(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ListText>,
) -> AppResult<Json<DkimSelectorsSummary>> {
    let (selectors, errors) = parse(&req.text);
    if !errors.is_empty() {
        return Err(rejected_lines(&errors, "list"));
    }
    let stored = get_custom_dkim_selectors(&state.pool).await?;
    let selectors: Vec<String> = selectors.into_iter().collect();
    set_custom_dkim_selectors(&state.pool, &selectors).await?;
    Ok(Json(DkimSelectorsSummary {
        changes: changes(&stored, &selectors.iter().cloned().collect()),
        errors: Vec::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_are_read_per_line_and_bad_ones_named() {
        let (selectors, errors) = parse("selector1, Google\n\nk1;s2._x\nbad$ ok\n");
        assert_eq!(
            selectors.into_iter().collect::<Vec<_>>(),
            ["google", "k1", "ok", "s2._x", "selector1"]
        );
        assert_eq!(
            errors,
            [LineError {
                line: 4,
                message: "not a DKIM selector: bad$".into()
            }]
        );
    }

    #[test]
    fn changes_name_what_is_added_and_removed() {
        let stored = vec!["a".to_owned(), "b".to_owned()];
        let c = changes(&stored, &["b".to_owned(), "c".to_owned()].into());
        assert_eq!(c.add, ["c"]);
        assert_eq!(c.remove, ["a"]);
    }
}
