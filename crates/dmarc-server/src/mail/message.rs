//! One message to send, and the images attached inline beside its HTML.

use dmarc_store::{PixelSize, StoredLogo};
use lettre::Address;
use std::fmt;

/// One message to send. `Reply-To` comes from the [`SmtpConfig`](super::SmtpConfig).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub to: Address,
    pub subject: String,
    pub text: String,
    /// An HTML alternative to `text`.
    pub html: Option<String>,
    /// The images the HTML shows by Content-ID (`cid:logo`), attached
    /// inline beside it (`multipart/related`). Ignored without `html`.
    pub images: Vec<InlineImage>,
}

/// The Content-ID the HTML of an email names the operator's logo by
/// (`<img src="cid:logo">`). Inline attachments, not `data:` URLs, which
/// Gmail and others strip.
pub const LOGO_CID: &str = "logo";

/// The Content-ID of Marcan's bar mark, beside "Marcan" atop an email
/// without an operator license.
pub const MARK_CID: &str = "marcan-mark";

/// Marcan's bar mark as a PNG, drawn from `web/src/lib/components/logo-mark.svelte`
/// at twice its 20 px size (the bars grey, the middle one blue) on a
/// transparent background.
const MARK_PNG: &[u8] = include_bytes!("marcan-mark.png");
/// [`MARK_PNG`]'s own size (a test reads it from the file).
const MARK_PNG_SIZE: PixelSize = PixelSize {
    width: 40,
    height: 40,
};

/// An image attached inline to a message.
#[derive(Clone, PartialEq, Eq)]
pub struct InlineImage {
    /// What the HTML names it by: `cid:<cid>`.
    pub cid: &'static str,
    /// `image/png`, `image/jpeg` or `image/webp`.
    pub media_type: &'static str,
    pub bytes: Vec<u8>,
    /// Its own size in pixels, from which the HTML's `width` and `height`
    /// attributes are computed ([`image_size::fit`](super::image_size::fit)).
    pub size: PixelSize,
}

impl InlineImage {
    /// The operator's uploaded logo ([`LOGO_CID`]) at the size stored with it.
    pub fn logo(stored: &StoredLogo) -> Self {
        Self {
            cid: LOGO_CID,
            media_type: stored.logo_type.media_type(),
            bytes: stored.bytes.clone(),
            size: stored.size,
        }
    }

    /// Marcan's bar mark ([`MARK_CID`]).
    pub fn marcan_mark() -> Self {
        Self {
            cid: MARK_CID,
            media_type: "image/png",
            bytes: MARK_PNG.to_vec(),
            size: MARK_PNG_SIZE,
        }
    }
}

impl fmt::Debug for InlineImage {
    /// The Content-ID, type and size, not the bytes.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "InlineImage({}, {}, {} bytes)",
            self.cid,
            self.media_type,
            self.bytes.len()
        )
    }
}
