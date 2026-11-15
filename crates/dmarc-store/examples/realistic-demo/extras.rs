//! Everything around the reports: the settings of a running installation
//! (a receiving and a sending account with example values and a placeholder
//! password each, sending off),
//! the operator's branding and logo, the client report recipients and the
//! months already sent, the senders' reverse DNS, and when each report
//! arrived.

use crate::catalogue::{
    forwarder, OrgSpec, FORWARDING_ORGS, OPERATOR, OPERATOR_DOMAIN, REPORT_ADDRESS,
};
use crate::services::Services;
use dmarc_store::{
    insert_account, record_client_report_send, replace_client_report_recipients, save_settings,
    set_logo, upsert_identity_snapshot, AccountFields, AlertSettings, BrandingSettings,
    ClientReportRecipient, ClientReportSettings, DigestFrequency, ImapSecurity, LogoType,
    OnboardingSettings, OnboardingState, PixelSize, ReceivingSettings, SecretKind, SecretRow,
    SendingSettings, SmtpSecurity, StoredLogo, Weekday, DEFAULT_CLIENT_REPORT_DAY,
    DEFAULT_CLIENT_REPORT_HOUR,
};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use sqlx::SqlitePool;
use std::collections::BTreeSet;
use std::io::Write;

/// The settings and accounts, the accounts added at `added`.
pub async fn settings(pool: &SqlitePool, added: i64) -> anyhow::Result<()> {
    save_settings(
        pool,
        &OnboardingSettings {
            state: Some(OnboardingState::Completed),
        },
    )
    .await?;
    // Each account has a placeholder password, so it reads as configured;
    // the server's checks fail against the made-up host (`.example` does
    // not resolve), and sending is off.
    let password = || [SecretRow::plain(SecretKind::Password, "demo-placeholder")];
    let mailbox = AccountFields {
        name: "Reports".to_owned(),
        address: REPORT_ADDRESS.to_owned(),
        username: REPORT_ADDRESS.to_owned(),
        imap_host: format!("imap.{OPERATOR_DOMAIN}"),
        imap_port: 993,
        imap_security: ImapSecurity::Tls,
        folder: "INBOX".to_owned(),
        forwarded_from: FORWARDING_ORGS.iter().map(|&org| forwarder(org)).collect(),
        ..AccountFields::default()
    };
    insert_account(pool, Some(1), &mailbox, &password(), added).await?;
    // Sending through an account of its own, switched off.
    let sender = AccountFields {
        name: "Notifications".to_owned(),
        address: format!("notifications@{OPERATOR_DOMAIN}"),
        username: format!("notifications@{OPERATOR_DOMAIN}"),
        smtp_host: format!("smtp.{OPERATOR_DOMAIN}"),
        smtp_port: Some(587),
        smtp_security: SmtpSecurity::StartTls,
        receive_enabled: false,
        ..AccountFields::default()
    };
    let sending_account = insert_account(pool, Some(2), &sender, &password(), added).await?;
    save_settings(
        pool,
        &ReceivingSettings {
            poll_interval_secs: 300,
        },
    )
    .await?;
    save_settings(
        pool,
        &SendingSettings {
            enabled: false,
            account_id: Some(sending_account),
            reply_to: format!("support@{OPERATOR_DOMAIN}"),
            test_recipient: format!("ops@{OPERATOR_DOMAIN}"),
        },
    )
    .await?;
    save_settings(
        pool,
        &AlertSettings {
            enabled: true,
            recipients: vec![format!("ops@{OPERATOR_DOMAIN}")],
            digest: DigestFrequency::Weekly,
            digest_hour: 7,
            digest_weekday: Weekday::Monday,
            base_url: format!("https://marcan.{OPERATOR_DOMAIN}"),
            muted_kinds: Vec::new(),
        },
    )
    .await?;
    save_settings(
        pool,
        &ClientReportSettings {
            enabled: true,
            day_of_month: DEFAULT_CLIENT_REPORT_DAY,
            hour_utc: DEFAULT_CLIENT_REPORT_HOUR,
            reply_to: format!("support@{OPERATOR_DOMAIN}"),
        },
    )
    .await?;
    save_settings(
        pool,
        &BrandingSettings {
            operator_name: OPERATOR.to_owned(),
        },
    )
    .await?;
    let (bytes, size) = logo();
    set_logo(
        pool,
        Some(&StoredLogo {
            logo_type: LogoType::Png,
            bytes,
            size,
        }),
    )
    .await?;
    Ok(())
}

/// The client report recipients, and the reports already sent to them this
/// month and the last two (the month a send is filed under is its own).
pub async fn client_reports(
    pool: &SqlitePool,
    orgs: &[(i64, &OrgSpec)],
    now: i64,
) -> anyhow::Result<usize> {
    let mut list = Vec::new();
    for (id, spec) in orgs {
        for email in spec.recipients {
            list.push(ClientReportRecipient {
                organization_id: *id,
                organization: spec.name.to_owned(),
                email: (*email).to_owned(),
            });
        }
    }
    let named: Vec<i64> = orgs.iter().map(|(id, _)| *id).collect();
    replace_client_report_recipients(pool, &named, &list).await?;

    let (year, month, day) = civil(now / 86_400);
    let this_month_sent = day > u32::from(DEFAULT_CLIENT_REPORT_DAY);
    for back in 0..3 {
        if back == 0 && !this_month_sent {
            continue;
        }
        let (y, m) = month_back(year, month, back);
        let sent_at = days_from_civil(y, m, u32::from(DEFAULT_CLIENT_REPORT_DAY)) * 86_400
            + i64::from(DEFAULT_CLIENT_REPORT_HOUR) * 3600
            + 95;
        for (i, r) in list.iter().enumerate() {
            let key = format!("{y:04}-{m:02}");
            record_client_report_send(
                pool,
                r.organization_id,
                &r.email,
                &key,
                sent_at + i as i64 * 4,
            )
            .await?;
        }
    }
    Ok(list.len())
}

/// Reverse DNS for every address the reports name. A documentation address
/// is never looked up by the server, so these stay as written.
pub async fn identities(
    pool: &SqlitePool,
    services: &Services,
    used: &BTreeSet<String>,
    now: i64,
) -> anyhow::Result<usize> {
    let mut n = 0;
    for host in services.all_hosts().filter(|h| used.contains(&h.ip)) {
        let checked_at = now - 3600 - (n as i64 * 37) % 20_000;
        let (name, confirmed) = match &host.ptr {
            Some((name, confirmed)) => (Some(name.as_str()), Some(*confirmed)),
            None => (None, None),
        };
        upsert_identity_snapshot(pool, &host.ip, checked_at, name, confirmed).await?;
        n += 1;
    }
    Ok(n)
}

/// Reports arrive a few hours after their day ends, not all at once.
pub async fn arrival_times(pool: &SqlitePool, now: i64) -> anyhow::Result<()> {
    sqlx::query("UPDATE reports SET created_at = MIN(date_end + 1800 + (id * 7919) % 30000, ?)")
        .bind(now - 300)
        .execute(pool)
        .await?;
    Ok(())
}

/// `(year, month, day)` of a day count since 1970-01-01 (H. Hinnant's
/// algorithm).
fn civil(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (yoe + era * 400 + i64::from(m <= 2)) as i32;
    (y, m, d)
}

fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn month_back(year: i32, month: u32, back: u32) -> (i32, u32) {
    let index = year * 12 + month as i32 - 1 - back as i32;
    (index.div_euclid(12), index.rem_euclid(12) as u32 + 1)
}

/// The "Example IT" logo: a teal mark and the name in block letters, drawn
/// here so the demo needs no image file.
pub fn logo() -> (Vec<u8>, PixelSize) {
    const W: usize = 480;
    const H: usize = 96;
    let mut px = vec![[0u8; 4]; W * H];
    let teal = [15, 118, 110, 255];
    let white = [255, 255, 255, 255];
    let ink = [30, 41, 59, 255];
    // The mark: a rounded square with an "E" cut out.
    let (mx, my, size, radius) = (12usize, 12usize, 72usize, 14.0f64);
    for y in 0..size {
        for x in 0..size {
            let cx = (x as f64 + 0.5).clamp(radius, size as f64 - radius);
            let cy = (y as f64 + 0.5).clamp(radius, size as f64 - radius);
            let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
            if dx * dx + dy * dy <= radius * radius {
                px[(my + y) * W + mx + x] = teal;
            }
        }
    }
    let bars = [
        (20, 16, 34, 9),
        (20, 32, 26, 8),
        (20, 47, 34, 9),
        (20, 16, 9, 40),
    ];
    for (x0, y0, w, h) in bars {
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                px[(my + y) * W + mx + x] = white;
            }
        }
    }
    // The name, 5×7 glyphs at six pixels each.
    let glyph = |c: char| -> [u8; 7] {
        match c {
            'E' => [31, 16, 16, 30, 16, 16, 31],
            'X' => [17, 17, 10, 4, 10, 17, 17],
            'A' => [14, 17, 17, 31, 17, 17, 17],
            'M' => [17, 27, 21, 21, 17, 17, 17],
            'P' => [30, 17, 17, 30, 16, 16, 16],
            'L' => [16, 16, 16, 16, 16, 16, 31],
            'I' => [14, 4, 4, 4, 4, 4, 14],
            'T' => [31, 4, 4, 4, 4, 4, 4],
            _ => [0; 7],
        }
    };
    let scale = 6;
    for (i, c) in "EXAMPLE IT".chars().enumerate() {
        let rows = glyph(c);
        let x0 = 104 + i * 6 * scale;
        let y0 = (H - 7 * scale) / 2;
        for (r, bits) in rows.iter().enumerate() {
            for col in 0..5 {
                if bits & (16 >> col) == 0 {
                    continue;
                }
                for y in 0..scale {
                    for x in 0..scale {
                        px[(y0 + r * scale + y) * W + x0 + col * scale + x] = ink;
                    }
                }
            }
        }
    }
    (
        png(W as u32, H as u32, &px),
        PixelSize {
            width: W as u32,
            height: H as u32,
        },
    )
}

/// An 8-bit RGBA PNG of `pixels` (rows top to bottom).
fn png(width: u32, height: u32, pixels: &[[u8; 4]]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(pixels.len() * 4 + height as usize);
    for row in pixels.chunks(width as usize) {
        raw.push(0); // filter: none
        for p in row {
            raw.extend_from_slice(p);
        }
    }
    let mut z = ZlibEncoder::new(Vec::new(), Compression::best());
    z.write_all(&raw).expect("compressing in memory");
    let idat = z.finish().expect("compressing in memory");

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, no interlace
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [(b"IHDR", &ihdr), (b"IDAT", &idat), (b"IEND", &Vec::new())] {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let crc = crc32(&out[start..]);
        out.extend_from_slice(&crc.to_be_bytes());
    }
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
