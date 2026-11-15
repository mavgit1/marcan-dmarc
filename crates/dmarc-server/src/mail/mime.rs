//! The one MIME form of a [`Message`]: what goes to an SMTP server after
//! `DATA` and, in base64, to Microsoft Graph's `sendMail` alike, so a
//! message is the same whichever way it leaves.

use super::{Message, SendConfig, SendError};
use lettre::message::{header::ContentType, Attachment, Mailbox, MultiPart, SinglePart};
use std::sync::atomic::{AtomicU64, Ordering};

/// The message as lettre sends it: `From` the sender, `To` its one
/// recipient (no `Cc`, no `Bcc`, so the headers name every address the
/// message goes to), `Reply-To` from the settings, plain text with an
/// optional HTML alternative, and its images beside the HTML as an inline
/// attachment (`multipart/related`).
pub(crate) fn build(config: &SendConfig, message: &Message) -> Result<lettre::Message, SendError> {
    let mut builder = lettre::Message::builder()
        .from(Mailbox::new(
            Some(config.sender_name.clone()),
            config.sender.clone(),
        ))
        .to(Mailbox::new(None, message.to.clone()))
        .subject(message.subject.as_str())
        .message_id(Some(message_id(config)));
    if let Some(reply_to) = &config.reply_to {
        builder = builder.reply_to(Mailbox::new(None, reply_to.clone()));
    }
    let built = match &message.html {
        Some(html) if !message.images.is_empty() => {
            let mut related = MultiPart::related().singlepart(SinglePart::html(html.clone()));
            for image in &message.images {
                let content_type =
                    ContentType::parse(image.media_type).map_err(|e| SendError::Other {
                        message: "Marcan could not put the message together.".into(),
                        detail: e.to_string(),
                    })?;
                related = related.singlepart(
                    Attachment::new_inline(image.cid.to_owned())
                        .body(image.bytes.clone(), content_type),
                );
            }
            builder.multipart(
                MultiPart::alternative()
                    .singlepart(SinglePart::plain(message.text.clone()))
                    .multipart(related),
            )
        }
        Some(html) => builder.multipart(MultiPart::alternative_plain_html(
            message.text.clone(),
            html.clone(),
        )),
        None => builder
            .header(ContentType::TEXT_PLAIN)
            .body(message.text.clone()),
    };
    built.map_err(|e| SendError::Other {
        message: "Marcan could not put the message together.".into(),
        detail: e.to_string(),
    })
}

/// A unique `Message-ID` in the sender's domain (not this host's name, which
/// inside a container means nothing to anyone).
fn message_id(config: &SendConfig) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    format!(
        "<{nanos:x}.{}.{:x}@{}>",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        config.sender.domain()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::smtp::tests::{config, message};
    use dmarc_store::SmtpSecurity;

    #[test]
    fn html_goes_out_as_an_alternative() {
        let cfg = config(465, SmtpSecurity::Tls);
        let email = build(
            &cfg,
            &Message {
                html: Some("<p>Hello</p>".into()),
                ..message()
            },
        )
        .unwrap();
        let text = String::from_utf8(email.formatted()).unwrap();
        assert!(text.contains("multipart/alternative"), "{text}");
        assert!(text.contains("text/html"), "{text}");
        assert!(!text.contains("multipart/related"), "{text}");
    }

    #[test]
    fn the_logo_is_attached_inline_beside_the_html() {
        let cfg = config(465, SmtpSecurity::Tls);
        let email = build(
            &cfg,
            &Message {
                html: Some("<p><img src=\"cid:logo\"><img src=\"cid:marcan-mark\"></p>".into()),
                images: vec![
                    crate::mail::InlineImage {
                        cid: "logo",
                        media_type: "image/png",
                        bytes: b"\x89PNG\r\n\x1a\n".to_vec(),
                        size: dmarc_store::PixelSize {
                            width: 1,
                            height: 1,
                        },
                    },
                    crate::mail::InlineImage::marcan_mark(),
                ],
                ..message()
            },
        )
        .unwrap();
        let text = String::from_utf8(email.formatted()).unwrap();
        // Plain text, then the HTML and its image together.
        assert!(text.contains("multipart/alternative"), "{text}");
        assert!(text.contains("multipart/related"), "{text}");
        assert!(text.contains("Content-ID: <logo>\r\n"), "{text}");
        assert!(text.contains("Content-ID: <marcan-mark>\r\n"), "{text}");
        assert!(text.contains("Content-Disposition: inline"), "{text}");
        assert!(text.contains("Content-Type: image/png\r\n"), "{text}");
        assert!(text.contains("iVBORw0KGgo="), "{text}");
        assert!(
            text.find("text/plain").unwrap() < text.find("multipart/related").unwrap(),
            "{text}"
        );

        // A plain-text message has no attachment, logo or not.
        let plain = build(
            &cfg,
            &Message {
                images: vec![crate::mail::InlineImage::marcan_mark()],
                ..message()
            },
        )
        .unwrap();
        let text = String::from_utf8(plain.formatted()).unwrap();
        assert!(!text.contains("multipart"), "{text}");
    }

    #[test]
    fn the_sender_name_is_the_from_display_name() {
        let cfg = SendConfig {
            sender_name: "Acme IT via Marcan".into(),
            ..config(465, SmtpSecurity::Tls)
        };
        let email = build(&cfg, &message()).unwrap();
        let text = String::from_utf8(email.formatted()).unwrap();
        assert!(
            text.contains("From: \"Acme IT via Marcan\" <dmarc@example.com>\r\n"),
            "{text}"
        );
        // The envelope sender stays the address.
        assert_eq!(
            email.envelope().from().map(ToString::to_string).as_deref(),
            Some("dmarc@example.com")
        );
    }
}
