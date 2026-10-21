//! How the emails present themselves: the letterhead (Marcan and the
//! operator's name and logo, as the license's standing decides), the From
//! name, the footer with the free-software notice, the colour of a line,
//! times and HTML escaping. What they say about an event or a domain's
//! readiness is [`crate::wording`]'s, shared with the UI.

use super::{image_size, InlineImage, LOGO_CID, MARK_CID};
use crate::license::{self, Standing, SOURCE_URL};
use crate::view::settings::{EmailTop, OperatorLetterhead, SoftwareLine};
use chrono::{DateTime, Utc};
use dmarc_assess::Status;
use dmarc_store::{AppSettings, PixelSize};

/// Who an email is from and whether it carries the free-software notice:
/// the branding (free for everyone) and the license's [`Standing`]. Every
/// email's top ([`email_top`]), its From name ([`from_name`]) and its last
/// line ([`sign_off`]) are decided from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Letterhead {
    /// In the letterhead and the From name.
    pub operator_name: Option<String>,
    /// In the HTML letterhead, attached inline and shown as `cid:logo`
    /// ([`super::LOGO_CID`]); the plain text has the name only.
    pub logo: Option<InlineImage>,
    /// [`Standing::notice_shown`]: Marcan opens the email and the
    /// free-software notice ends it, the operator's letterhead secondary.
    pub notice: bool,
}

impl Default for Letterhead {
    /// No branding, the notice shown: what the license asks for by default.
    fn default() -> Self {
        Self {
            operator_name: None,
            logo: None,
            notice: true,
        }
    }
}

/// The box the operator's logo is fitted into atop an email with an
/// operator license (width, height in pixels): wide text logos stay readable.
pub const LOGO_BOX: (u32, u32) = (200, 64);
/// The box without one, under Marcan's line.
pub const LOGO_SECONDARY_BOX: (u32, u32) = (120, 32);
/// The size the Marcan mark is shown at, beside the wordmark.
pub const MARK_SIZE: PixelSize = PixelSize {
    width: 20,
    height: 20,
};

/// The top of every email, from the operator's name, the logo's own size
/// (`None` without a logo) and whether the free-software notice is shown
/// ([`Standing::notice_shown`], the one switch). With the notice: Marcan
/// first, as free software, then the operator's letterhead at reduced
/// prominence, the logo fitted into [`LOGO_SECONDARY_BOX`] and "Managed by
/// <name>" (none without a name). Without it: the logo fitted into
/// [`LOGO_BOX`] and the name in bold, nothing naming Marcan. The emails
/// ([`Letterhead`]) and the Branding page's preview both ask here.
pub fn email_top(operator_name: Option<&str>, logo: Option<PixelSize>, notice: bool) -> EmailTop {
    let name = operator_name.map(str::trim).filter(|n| !n.is_empty());
    let fitted = |(w, h): (u32, u32)| logo.map(|size| image_size::fit(size, w, h));
    if notice {
        EmailTop {
            software: Some(SoftwareLine {
                mark: MARK_SIZE,
                wordmark: "Marcan".to_owned(),
                rest: ", free software".to_owned(),
            }),
            operator: name.map(|name| OperatorLetterhead {
                prominent: false,
                logo: fitted(LOGO_SECONDARY_BOX),
                name: Some(format!("Managed by {name}")),
            }),
        }
    } else {
        EmailTop {
            software: None,
            operator: (name.is_some() || logo.is_some()).then(|| OperatorLetterhead {
                prominent: true,
                logo: fitted(LOGO_BOX),
                name: name.map(str::to_owned),
            }),
        }
    }
}

/// `width="W" height="H"` and the same as CSS, so every client shows an
/// image at that size: desktop Outlook reads only the attributes.
fn img_size(size: PixelSize) -> (String, String) {
    let PixelSize { width, height } = size;
    (
        format!("width=\"{width}\" height=\"{height}\""),
        format!("width:{width}px;height:{height}px"),
    )
}

/// The operator's letterhead in the HTML: the logo left of the name in one
/// row, both vertically centred, as a one-row table (email clients centre
/// table cells reliably, mixed inline content not); the name alone as a
/// paragraph. Prominent with an operator license (the name in bold),
/// secondary under Marcan's line without one.
fn operator_row(operator: &OperatorLetterhead) -> String {
    let name = operator.name.as_deref().unwrap_or_default();
    let (margin, gap, text) = if operator.prominent {
        (8, 12, "font-size:20px;font-weight:700;color:#111827")
    } else {
        (12, 8, "font-size:13px;color:#525252")
    };
    let Some(size) = operator.logo else {
        return if operator.prominent {
            format!(
                "<p style=\"margin:0 0 4px;font-size:20px;font-weight:700\">{}</p>",
                escape(name)
            )
        } else {
            format!(
                "<p style=\"margin:0 0 12px;font-size:13px;color:#525252\">{}</p>",
                escape(name)
            )
        };
    };
    let (attrs, css) = img_size(size);
    // The name beside it says who it is; alone, the logo has nothing to name.
    let name_cell = if name.is_empty() {
        String::new()
    } else {
        format!(
            "<td style=\"vertical-align:middle;padding:0 0 0 {gap}px;{text}\">{}</td>",
            escape(name)
        )
    };
    format!(
        "<table role=\"presentation\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" \
         style=\"border-collapse:collapse;margin:0 0 {margin}px\"><tr>\
         <td style=\"vertical-align:middle;padding:0\"><img src=\"cid:{LOGO_CID}\" alt=\"\" \
         {attrs} style=\"{css};display:block;border:0\"></td>{name_cell}</tr></table>"
    )
}

/// The `From` display name of every email (the address is the sender's):
/// "<name> via Marcan" while the notice is shown, else "<name>"; "Marcan"
/// without a name either way.
pub fn from_name(operator_name: Option<&str>, notice: bool) -> String {
    match operator_name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) if notice => format!("{name} via Marcan"),
        Some(name) => name.to_owned(),
        None => "Marcan".to_owned(),
    }
}

impl Letterhead {
    /// The letterhead of `settings`, without the logo ([`Self::load`] adds it).
    pub fn new(settings: &AppSettings) -> Self {
        Self {
            operator_name: settings.branding.name().map(str::to_owned),
            logo: None,
            notice: Standing::new(&settings.license).notice_shown,
        }
    }

    /// The letterhead of `settings` with the uploaded logo, which is stored
    /// apart from the settings. A logo that cannot be read is left out (and
    /// logged): the email still goes out.
    pub async fn load(pool: &sqlx::SqlitePool, settings: &AppSettings) -> Self {
        let logo = dmarc_store::get_logo(pool).await.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not read the logo; the email goes without it");
            None
        });
        Self {
            logo: logo.as_ref().map(InlineImage::logo),
            ..Self::new(settings)
        }
    }

    /// The top of the email ([`email_top`]).
    pub fn top(&self) -> EmailTop {
        email_top(
            self.operator_name.as_deref(),
            self.logo.as_ref().map(|l| l.size),
            self.notice,
        )
    }

    /// The `From` display name ([`from_name`]).
    pub fn from_name(&self) -> String {
        from_name(self.operator_name.as_deref(), self.notice)
    }

    /// The images the HTML top shows, to attach inline: the Marcan mark
    /// beside its line, the logo when the operator's letterhead shows it.
    pub(crate) fn images(&self) -> Vec<InlineImage> {
        let top = self.top();
        let mut images = Vec::new();
        if top.software.is_some() {
            images.push(InlineImage::marcan_mark());
        }
        if top.operator.is_some_and(|o| o.logo.is_some()) {
            images.extend(self.logo.clone());
        }
        images
    }

    /// The top of the plain text: the letterhead's lines and a blank line
    /// (none without a letterhead), then `title` and a blank line.
    pub(crate) fn text_heading(&self, title: &str) -> String {
        let top = self.top();
        let mut lines = Vec::new();
        if let Some(software) = &top.software {
            lines.push(format!("{}{}", software.wordmark, software.rest));
        }
        lines.extend(top.operator.and_then(|o| o.name));
        let mut out = String::new();
        if !lines.is_empty() {
            out = lines.join("\n");
            out.push_str("\n\n");
        }
        out.push_str(title);
        out.push_str("\n\n");
        out
    }

    /// The top of the HTML: the letterhead ([`email_top`]), the images by
    /// their Content-ID, then `title` as the heading of the content.
    pub(crate) fn html_heading(&self, title: &str) -> String {
        let top = self.top();
        let mut out = String::new();
        if let Some(software) = &top.software {
            let (attrs, css) = img_size(software.mark);
            out.push_str(&format!(
                "<p style=\"margin:0 0 12px;font-size:13px;line-height:{}px;color:#737373\">\
                 <img src=\"cid:{MARK_CID}\" alt=\"\" {attrs} \
                 style=\"{css};vertical-align:middle;border:0\">&nbsp;\
                 <span style=\"font-weight:600;vertical-align:middle\">{}</span>\
                 <span style=\"vertical-align:middle\">{}</span></p>",
                software.mark.height,
                escape(&software.wordmark),
                escape(&software.rest)
            ));
        }
        if let Some(operator) = &top.operator {
            out.push_str(&operator_row(operator));
        }
        out.push_str(&format!(
            "<h1 style=\"font-size:16px;font-weight:600;margin:0 0 8px\">{}</h1>",
            escape(title)
        ));
        out
    }
}

/// The end of an email, as plain text and as HTML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Footer {
    pub text: String,
    pub html: String,
}

/// The last sentence of every email: the free-software notice while
/// `notice` is shown ([`license::notice`], signed by the operator when one
/// is named), else "Sent by <operator name>." (or "Sent by Marcan.").
pub fn sign_off(operator_name: Option<&str>, notice: bool) -> String {
    if notice {
        license::notice(operator_name)
    } else {
        format!("Sent by {}.", operator_name.unwrap_or("Marcan"))
    }
}

/// The end of every email Marcan sends: `about` (where to change it, or
/// what the email is, in the reader's terms), then the [`sign_off`]. The one
/// place the notice goes into alert, digest and client-report emails.
pub fn footer(about: &str, operator_name: Option<&str>, notice: bool) -> Footer {
    let sign_off = sign_off(operator_name, notice);
    let link = format!(
        "<a href=\"{0}\" style=\"color:#737373\">{0}</a>",
        escape(SOURCE_URL)
    );
    let mut sign_off_html = escape(&sign_off);
    if notice {
        sign_off_html = sign_off_html.replace(&escape(SOURCE_URL), &link);
    }
    Footer {
        text: format!("{about}\n\n{sign_off}"),
        html: format!("{}<br><br>{sign_off_html}", escape(about)),
    }
}

/// The footer's `about` in the emails to whoever runs Marcan (alerts, digest).
pub(crate) const ALERTS_FOOTER: &str =
    "Recipients, the digest and alert emails are set under Settings → Alerts.";

/// The footer's `about` in a client report: its reader has no access to Marcan.
pub(crate) const CLIENT_REPORT_FOOTER: &str =
    "This report shows how mail sent from your domains is authenticated (DMARC).";

/// The colour of a line in the HTML email (text, then a light background).
pub(crate) fn status_colours(status: Status) -> (&'static str, &'static str) {
    match status {
        Status::Fail => ("#b91c1c", "#fef2f2"),
        Status::Warn => ("#b45309", "#fffbeb"),
        Status::Pass => ("#15803d", "#f0fdf4"),
        Status::Info => ("#1d4ed8", "#eff6ff"),
        Status::Forward => ("#6d28d9", "#f5f3ff"),
        Status::Neutral => ("#525252", "#f5f5f5"),
    }
}

/// `2026-10-13 07:00 UTC`.
pub(crate) fn format_time(unix: i64) -> String {
    DateTime::<Utc>::from_timestamp(unix, 0)
        .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_default()
}

/// `2026-10-13`.
pub(crate) fn format_date(unix: i64) -> String {
    DateTime::<Utc>::from_timestamp(unix, 0)
        .map(|t| t.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// `text` safe inside HTML.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// `Acme's`, `Beta Holdings'`.
pub fn possessive(name: &str) -> String {
    if name.ends_with('s') || name.ends_with('S') {
        format!("{name}'")
    } else {
        format!("{name}'s")
    }
}

/// `September 2026`.
pub(crate) fn format_month(unix: i64) -> String {
    DateTime::<Utc>::from_timestamp(unix, 0)
        .map(|t| t.format("%B %Y").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_footer_ends_in_the_notice_unless_it_is_off() {
        let shown = footer("It shows DMARC.", None, true);
        assert_eq!(
            shown.text,
            concat!("It shows DMARC.\n\nSent by Marcan, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: https://github.com/mavgit1/marcan-dmarc/tree/v", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(
            shown.html,
            concat!("It shows DMARC.<br><br>Sent by Marcan, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: <a href=\"https://github.com/mavgit1/marcan-dmarc/tree/v", env!("CARGO_PKG_VERSION"), "\" style=\"color:#737373\">https://github.com/mavgit1/marcan-dmarc/tree/v", env!("CARGO_PKG_VERSION"), "</a>")
        );
        // The operator signs the notice when branding names one.
        let named = footer("It shows DMARC.", Some("Acme & Co"), true);
        assert!(named.text.ends_with(
            concat!("\n\nSent by Acme & Co with Marcan, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: https://github.com/mavgit1/marcan-dmarc/tree/v", env!("CARGO_PKG_VERSION"))
        ));
        assert!(named
            .html
            .contains("<br><br>Sent by Acme &amp; Co with Marcan, free software"));
        // Off: the operator alone, or Marcan as the product without a name.
        let off = footer("It shows <DMARC>.", Some("Acme <IT>"), false);
        assert_eq!(off.text, "It shows <DMARC>.\n\nSent by Acme <IT>.");
        assert_eq!(
            off.html,
            "It shows &lt;DMARC&gt;.<br><br>Sent by Acme &lt;IT&gt;."
        );
        assert_eq!(footer("x", None, false).text, "x\n\nSent by Marcan.");
    }

    #[test]
    fn the_letterhead_follows_the_settings() {
        use dmarc_store::{BrandingSettings, LicenseSettings};
        assert_eq!(
            Letterhead::new(&AppSettings::default()),
            Letterhead::default()
        );
        assert!(Letterhead::default().notice);
        let settings = AppSettings {
            branding: BrandingSettings {
                operator_name: "Acme IT".into(),
            },
            // A key that does not verify: the notice is still shown.
            license: LicenseSettings {
                operator_key: Some("test@example.com.AAAA".into()),
            },
            ..AppSettings::default()
        };
        let letterhead = Letterhead::new(&settings);
        assert_eq!(letterhead.operator_name.as_deref(), Some("Acme IT"));
        assert!(letterhead.notice);
        assert_eq!(
            letterhead.text_heading("Title"),
            "Marcan, free software\nManaged by Acme IT\n\nTitle\n\n"
        );
        assert_eq!(
            Letterhead::default().text_heading("Title"),
            "Marcan, free software\n\nTitle\n\n"
        );
        let licensed = Letterhead {
            notice: false,
            ..letterhead
        };
        assert_eq!(licensed.text_heading("Title"), "Acme IT\n\nTitle\n\n");
        // Licensed without a name: the title alone.
        let bare = Letterhead {
            operator_name: None,
            ..licensed
        };
        assert_eq!(bare.text_heading("Title"), "Title\n\n");
        assert_eq!(
            bare.html_heading("A <b>"),
            "<h1 style=\"font-size:16px;font-weight:600;margin:0 0 8px\">A &lt;b&gt;</h1>"
        );
        assert!(bare.images().is_empty());
    }

    fn px(width: u32, height: u32) -> PixelSize {
        PixelSize { width, height }
    }

    /// A wide text logo, 1200 by 300 pixels.
    fn logo() -> InlineImage {
        InlineImage {
            cid: LOGO_CID,
            media_type: "image/png",
            bytes: b"\x89PNG\r\n\x1a\n".to_vec(),
            size: px(1200, 300),
        }
    }

    /// Without a license Marcan comes first and the operator second, with
    /// its logo fitted into 120 by 32 px and "Managed by"; with one the
    /// operator alone, the logo fitted into 200 by 64 px and the name in bold.
    #[test]
    fn the_top_follows_the_license() {
        let software = Some(SoftwareLine {
            mark: px(20, 20),
            wordmark: "Marcan".into(),
            rest: ", free software".into(),
        });
        let wide = Some(px(1200, 300));
        assert_eq!(
            email_top(Some("Acme IT"), wide, true),
            EmailTop {
                software: software.clone(),
                operator: Some(OperatorLetterhead {
                    prominent: false,
                    logo: Some(px(120, 30)),
                    name: Some("Managed by Acme IT".into()),
                }),
            }
        );
        // A square logo is held by the box's height.
        assert_eq!(
            email_top(Some("Acme IT"), Some(px(512, 512)), true)
                .operator
                .and_then(|o| o.logo),
            Some(px(32, 32))
        );
        // No name: no "Managed by" line, logo or not.
        assert_eq!(
            email_top(None, wide, true),
            EmailTop {
                software,
                operator: None
            }
        );
        assert_eq!(
            email_top(Some(" "), None, true).operator,
            None,
            "a blank name is none"
        );
        assert_eq!(
            email_top(Some("Acme IT"), wide, false),
            EmailTop {
                software: None,
                operator: Some(OperatorLetterhead {
                    prominent: true,
                    logo: Some(px(200, 50)),
                    name: Some("Acme IT".into()),
                }),
            }
        );
        assert_eq!(
            email_top(None, Some(px(512, 512)), false).operator,
            Some(OperatorLetterhead {
                prominent: true,
                logo: Some(px(64, 64)),
                name: None,
            })
        );
        assert_eq!(
            email_top(None, None, false),
            EmailTop {
                software: None,
                operator: None
            }
        );
    }

    #[test]
    fn the_html_top_in_both_states() {
        let unlicensed = Letterhead {
            operator_name: Some("Acme & Co".into()),
            logo: Some(logo()),
            notice: true,
        };
        let html = unlicensed.html_heading("DMARC report for Beta, September 2026");
        // Every image carries its size as attributes (desktop Outlook reads
        // only those) and as CSS.
        assert_eq!(
            html,
            "<p style=\"margin:0 0 12px;font-size:13px;line-height:20px;color:#737373\">\
             <img src=\"cid:marcan-mark\" alt=\"\" width=\"20\" height=\"20\" style=\"width:20px;height:20px;vertical-align:middle;border:0\">&nbsp;\
             <span style=\"font-weight:600;vertical-align:middle\">Marcan</span><span style=\"vertical-align:middle\">, free software</span></p>\
             <table role=\"presentation\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"border-collapse:collapse;margin:0 0 12px\"><tr>\
             <td style=\"vertical-align:middle;padding:0\"><img src=\"cid:logo\" alt=\"\" width=\"120\" height=\"30\" style=\"width:120px;height:30px;display:block;border:0\"></td>\
             <td style=\"vertical-align:middle;padding:0 0 0 8px;font-size:13px;color:#525252\">Managed by Acme &amp; Co</td></tr></table>\
             <h1 style=\"font-size:16px;font-weight:600;margin:0 0 8px\">DMARC report for Beta, September 2026</h1>"
        );
        let cids: Vec<&str> = unlicensed.images().iter().map(|i| i.cid).collect();
        assert_eq!(cids, [MARK_CID, LOGO_CID]);
        let mark = InlineImage::marcan_mark();
        assert_eq!(
            image_size::pixel_size(&mark.bytes),
            Some(mark.size),
            "the mark's stated size is the file's"
        );

        let licensed = Letterhead {
            notice: false,
            ..unlicensed.clone()
        };
        let html = licensed.html_heading("DMARC report for Beta, September 2026");
        assert_eq!(
            html,
            "<table role=\"presentation\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"border-collapse:collapse;margin:0 0 8px\"><tr>\
             <td style=\"vertical-align:middle;padding:0\"><img src=\"cid:logo\" alt=\"\" width=\"200\" height=\"50\" style=\"width:200px;height:50px;display:block;border:0\"></td>\
             <td style=\"vertical-align:middle;padding:0 0 0 12px;font-size:20px;font-weight:700;color:#111827\">Acme &amp; Co</td></tr></table>\
             <h1 style=\"font-size:16px;font-weight:600;margin:0 0 8px\">DMARC report for Beta, September 2026</h1>"
        );
        assert!(!html.contains("Marcan"));
        let cids: Vec<&str> = licensed.images().iter().map(|i| i.cid).collect();
        assert_eq!(cids, [LOGO_CID]);

        // Licensed with a logo and no name: the logo alone in its row.
        let logo_only = Letterhead {
            operator_name: None,
            ..licensed
        };
        assert!(logo_only.html_heading("T").starts_with(
            "<table role=\"presentation\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"border-collapse:collapse;margin:0 0 8px\"><tr>\
             <td style=\"vertical-align:middle;padding:0\"><img src=\"cid:logo\" alt=\"\" width=\"200\" height=\"50\" style=\"width:200px;height:50px;display:block;border:0\"></td></tr></table><h1"
        ));

        // Unlicensed without a name: Marcan's line, and the logo is not attached.
        let nameless = Letterhead {
            operator_name: None,
            ..unlicensed
        };
        assert!(!nameless.html_heading("T").contains("cid:logo"));
        let cids: Vec<&str> = nameless.images().iter().map(|i| i.cid).collect();
        assert_eq!(cids, [MARK_CID]);
    }

    /// A stored logo is attached with the size stored with it.
    #[test]
    fn a_stored_logo_has_its_size() {
        use dmarc_store::{LogoType, StoredLogo};
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\x02\x80\0\0\0\xa0\x08\x06\0\0\0";
        let logo = InlineImage::logo(&StoredLogo {
            logo_type: LogoType::Png,
            bytes: png.to_vec(),
            size: px(640, 160),
        });
        assert_eq!(logo.size, px(640, 160));
        assert_eq!(logo.bytes, png);
        assert_eq!(logo.media_type, "image/png");
    }

    #[test]
    fn the_from_name_follows_the_license() {
        assert_eq!(from_name(Some("Acme IT"), true), "Acme IT via Marcan");
        assert_eq!(from_name(Some("Acme IT"), false), "Acme IT");
        assert_eq!(from_name(None, true), "Marcan");
        assert_eq!(from_name(None, false), "Marcan");
        assert_eq!(from_name(Some(" "), true), "Marcan");
        let licensed = Letterhead {
            operator_name: Some("Acme IT".into()),
            logo: None,
            notice: false,
        };
        assert_eq!(licensed.from_name(), "Acme IT");
    }

    #[test]
    fn html_is_escaped() {
        assert_eq!(
            escape("<a href=\"x\">&'"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
        assert_eq!(format_time(1_791_961_200), "2026-10-14 07:00 UTC");
        assert_eq!(format_month(1_791_961_200), "October 2026");
    }

    #[test]
    fn possessives() {
        assert_eq!(possessive("Acme"), "Acme's");
        assert_eq!(possessive("Beta Holdings"), "Beta Holdings'");
    }
}
