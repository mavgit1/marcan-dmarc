//! Parse the shared `since` query parameter.

use crate::error::AppError;

/// Parse `since` as `7d` / `30d` / `90d` (or any `Nd`) or a raw unix timestamp.
///
/// Returns `Ok(None)` when the parameter is absent or empty.
/// Returns `Ok(Some(unix_secs))` as the lower bound for `date_begin`.
pub fn parse_since(since: Option<&str>) -> Result<Option<i64>, AppError> {
    let Some(raw) = since.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };

    if let Some(days_str) = raw.strip_suffix('d') {
        let days: i64 = days_str.parse().map_err(|_| {
            AppError::bad_request(format!(
                "invalid since value '{raw}': expected Nd (e.g. 7d, 30d, 90d) or a unix timestamp"
            ))
        })?;
        if days < 0 {
            return Err(AppError::bad_request(format!(
                "invalid since value '{raw}': day count must be non-negative"
            )));
        }
        let now = dmarc_store::unix_now();
        return Ok(Some(now.saturating_sub(days.saturating_mul(86_400))));
    }

    let ts: i64 = raw.parse().map_err(|_| {
        AppError::bad_request(format!(
            "invalid since value '{raw}': expected Nd (e.g. 7d, 30d, 90d) or a unix timestamp"
        ))
    })?;
    Ok(Some(ts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn empty_and_none() {
        assert_eq!(parse_since(None).unwrap(), None);
        assert_eq!(parse_since(Some("")).unwrap(), None);
        assert_eq!(parse_since(Some("  ")).unwrap(), None);
    }

    #[test]
    fn relative_days() {
        let ts = parse_since(Some("7d")).unwrap().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!((now - 7 * 86_400 - ts).abs() <= 1);
    }

    #[test]
    fn raw_timestamp() {
        assert_eq!(
            parse_since(Some("1704067200")).unwrap(),
            Some(1_704_067_200)
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_since(Some("yesterday")).is_err());
        assert!(parse_since(Some("d7")).is_err());
        assert!(parse_since(Some("-1d")).is_err());
    }
}
