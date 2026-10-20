//! Who runs this Marcan: the operator's name, and the logo stored apart
//! (`branding_logo`).

use super::SettingsGroup;
use crate::StoreError;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;
use ts_rs::TS;

/// An image's size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PixelSize {
    pub width: u32,
    pub height: u32,
}

/// The image formats a logo may have. The server tells them apart by their
/// first bytes; this is only the stored name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum LogoType {
    Png,
    Jpeg,
    Webp,
}

impl LogoType {
    /// The media type, as stored and as a data URL or a MIME part names it.
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "image/png" => Some(Self::Png),
            "image/jpeg" => Some(Self::Jpeg),
            "image/webp" => Some(Self::Webp),
            _ => None,
        }
    }
}

/// The uploaded logo as stored in `branding_logo`: the image and its type.
/// The server checked the bytes and their size before storing them.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredLogo {
    pub logo_type: LogoType,
    pub bytes: Vec<u8>,
    /// Its size in pixels, read from its header at upload.
    pub size: PixelSize,
}

impl std::fmt::Debug for StoredLogo {
    /// The type and size, not the bytes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "StoredLogo({:?}, {} bytes)",
            self.logo_type,
            self.bytes.len()
        )
    }
}

/// The stored logo's type, size in bytes and size in pixels, read without
/// the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogoInfo {
    pub logo_type: LogoType,
    pub bytes: u32,
    pub size: PixelSize,
}

/// The uploaded logo; `None` without one (or with a type this version does
/// not know). Loaded only where the image is used: [`AppSettings`](super::AppSettings) holds no
/// logo, so loading the settings never reads it.
pub async fn get_logo(pool: &SqlitePool) -> Result<Option<StoredLogo>, StoreError> {
    let row: Option<(String, u32, u32, Vec<u8>)> =
        sqlx::query_as("SELECT media_type, width, height, image FROM branding_logo WHERE id = 1")
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|(media_type, width, height, bytes)| {
        LogoType::parse(&media_type).map(|logo_type| StoredLogo {
            logo_type,
            bytes,
            size: PixelSize { width, height },
        })
    }))
}

/// The stored logo's type and size, without reading the image.
pub async fn get_logo_info(pool: &SqlitePool) -> Result<Option<LogoInfo>, StoreError> {
    let row: Option<(String, i64, u32, u32)> = sqlx::query_as(
        "SELECT media_type, length(image), width, height FROM branding_logo WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|(media_type, bytes, width, height)| {
        LogoType::parse(&media_type).map(|logo_type| LogoInfo {
            logo_type,
            bytes: u32::try_from(bytes).unwrap_or(u32::MAX),
            size: PixelSize { width, height },
        })
    }))
}

/// Replace the logo (`Some`) or remove it (`None`; none is not an error).
pub async fn set_logo(pool: &SqlitePool, logo: Option<&StoredLogo>) -> Result<(), StoreError> {
    match logo {
        Some(logo) => {
            sqlx::query(
                "INSERT INTO branding_logo (id, media_type, width, height, image) \
                 VALUES (1, ?, ?, ?, ?) \
                 ON CONFLICT(id) DO UPDATE SET \
                 media_type = excluded.media_type, width = excluded.width, \
                 height = excluded.height, image = excluded.image",
            )
            .bind(logo.logo_type.media_type())
            .bind(logo.size.width)
            .bind(logo.size.height)
            .bind(&logo.bytes)
            .execute(pool)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM branding_logo")
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

/// Who runs this Marcan, shown in the emails, as stored and as `PUT
/// /api/settings/branding` takes it: free for everyone. The server checks the
/// name before saving. The logo is stored apart ([`get_logo`]) and has its
/// own endpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BrandingSettings {
    /// One line, at most 80 characters, saved trimmed. Empty: none.
    pub operator_name: String,
}

impl SettingsGroup for BrandingSettings {
    const KEY: &'static str = "branding";
}

impl BrandingSettings {
    /// The operator's name; `None` when not set.
    pub fn name(&self) -> Option<&str> {
        Some(self.operator_name.as_str()).filter(|n| !n.is_empty())
    }
}
