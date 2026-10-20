//! The frame every email shares: the letterhead and title on top, the body,
//! the [`footer`] at the end, in plain text and HTML side by side
//! ([`finish`]), and the rendered result ([`Content`]) that each recipient's
//! [`Message`] is made from. The alert email, the digest and the client
//! report render only their bodies.

use super::text::{escape, footer, Letterhead};
use super::{Address, InlineImage, Message};

/// A rendered email, ready to address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Content {
    pub subject: String,
    pub text: String,
    pub html: String,
    /// The letterhead's images, which `html` shows by Content-ID.
    pub images: Vec<InlineImage>,
}

impl Content {
    /// `html` with each inline image (`cid:...`) as a `data:` URL, for a
    /// page that shows the email itself (the attachments travel only in a
    /// message).
    pub fn html_with_data_urls(&self) -> String {
        use base64::Engine;
        self.images.iter().fold(self.html.clone(), |html, image| {
            let url = format!(
                "data:{};base64,{}",
                image.media_type,
                base64::engine::general_purpose::STANDARD.encode(&image.bytes)
            );
            html.replace(&format!("\"cid:{}\"", image.cid), &format!("\"{url}\""))
        })
    }

    /// The email for one recipient.
    pub fn message(&self, to: Address) -> Message {
        Message {
            to,
            subject: self.subject.clone(),
            text: self.text.clone(),
            html: Some(self.html.clone()),
            images: self.images.clone(),
        }
    }
}

/// The page around an HTML body: the letterhead and `title` on top
/// ([`Letterhead::html_heading`]), `footer` (already HTML) at the end;
/// plain, no external assets (the images are inline attachments).
pub(crate) fn html_page(
    subject: &str,
    title: &str,
    letterhead: &Letterhead,
    body: &str,
    footer: &str,
) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title></head>\
         <body style=\"font-family:-apple-system,Segoe UI,Helvetica,Arial,sans-serif;font-size:14px;color:#111827;max-width:640px;margin:0 auto;padding:16px\">\
         {}{body}<p style=\"color:#737373;font-size:12px;margin-top:24px\">{footer}</p></body></html>",
        escape(subject),
        letterhead.html_heading(title),
    )
}

/// What an email renders itself: its title and its body in plain text and
/// HTML side by side, for [`finish`] to frame.
pub(crate) struct Body<'a> {
    pub title: &'a str,
    pub text: &'a str,
    pub html: &'a str,
}

/// The whole email around a rendered body: the letterhead and the body's
/// title on top, then the body, then the [`footer`] with `about` (and the
/// notice, when shown).
pub(crate) fn finish(
    subject: String,
    letterhead: &Letterhead,
    body: &Body<'_>,
    about: &str,
) -> Content {
    let Body {
        title,
        text: body_text,
        html: body_html,
    } = *body;
    let footer = footer(
        about,
        letterhead.operator_name.as_deref(),
        letterhead.notice,
    );
    let text = format!(
        "{}{body_text}\n-- \n{}\n",
        letterhead.text_heading(title),
        footer.text
    );
    Content {
        html: html_page(&subject, title, letterhead, body_html, &footer.html),
        subject,
        text,
        images: letterhead.images(),
    }
}
