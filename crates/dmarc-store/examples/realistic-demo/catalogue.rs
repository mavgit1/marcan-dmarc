//! Every name the demo uses. Organizations, their domains and the small
//! hosting providers are invented; domains are on the reserved `.example`
//! TLD (RFC 2606). The only real names are generic mail services: the
//! receivers that send DMARC reports (every report carries one) and the large
//! platforms senders are identified as (Microsoft 365, Google, the security
//! gateways and mailing services in Marcan's bundled sender-name map).

/// The operator running this Marcan, as Settings → Branding names it.
pub const OPERATOR: &str = "Example IT";
/// The operator's domain: the report address and the made-up mail accounts.
pub const OPERATOR_DOMAIN: &str = "example-it.example";
/// Where every domain's aggregate reports go (`rua=`).
pub const REPORT_ADDRESS: &str = "dmarc-reports@example-it.example";

/// One client organization and its domains, main domain first.
pub struct OrgSpec {
    pub name: &'static str,
    /// Domains whose traffic follows the baseline's shape.
    pub shaped: &'static [&'static str],
    /// Extra domains in a clean state (p=reject, grade A).
    pub clean: &'static [&'static str],
    /// Domains that send no mail.
    pub parked: &'static [&'static str],
    /// Who gets the monthly client report (none: not listed).
    pub recipients: &'static [&'static str],
}

pub const ORGS: [OrgSpec; 9] = [
    OrgSpec {
        name: "Nordlicht Logistik",
        shaped: &[
            "nordlicht-logistik.example",
            "nordlicht-cargo.example",
            "nordlicht-lager.example",
            "nordlicht-group.example",
            "nl-spedition.example",
            "nordlicht-transporte.example",
            "nordlicht-karriere.example",
        ],
        clean: &[],
        parked: &["nordlicht-logistic.example"],
        recipients: &[
            "it-leitung@nordlicht-logistik.example",
            "c.brandt@nordlicht-logistik.example",
        ],
    },
    OrgSpec {
        name: "Agentur Wolkenwerk",
        shaped: &[
            "wolkenwerk.example",
            "wolkenwerk-studio.example",
            "wolkenwerk-shop.example",
            "wolkenwerk-events.example",
            "ww-media.example",
            "wolkenwerk-labs.example",
        ],
        clean: &[],
        parked: &["wolkenwerk-alt.example"],
        recipients: &["office@wolkenwerk.example"],
    },
    OrgSpec {
        name: "Kranich & Partner Treuhand",
        shaped: &[
            "kranich-treuhand.example",
            "kranich-partner.example",
            "kranich-steuern.example",
            "kp-revision.example",
        ],
        clean: &["kranich-portal.example"],
        parked: &[],
        recipients: &["m.kranich@kranich-treuhand.example"],
    },
    OrgSpec {
        name: "Seeblick Immobilien",
        shaped: &[
            "seeblick-immobilien.example",
            "seeblick-verwaltung.example",
            "seeblick-mieter.example",
            "wohnen-am-see.example",
            "seeblick-ferien.example",
        ],
        clean: &[],
        parked: &["seeblick-immo.example"],
        recipients: &[],
    },
    OrgSpec {
        name: "Bergquell Getränke",
        shaped: &[
            "bergquell.example",
            "bergquell-shop.example",
            "bergquell-gastro.example",
            "quellwasser-direkt.example",
        ],
        clean: &["bergquell-b2b.example"],
        parked: &[],
        recipients: &["informatik@bergquell.example"],
    },
    OrgSpec {
        name: "Stiftung Morgenrot",
        shaped: &[
            "stiftung-morgenrot.example",
            "morgenrot-spenden.example",
            "morgenrot-jugend.example",
            "morgenrot-kultur.example",
        ],
        clean: &[],
        parked: &[],
        recipients: &["geschaeftsstelle@stiftung-morgenrot.example"],
    },
    OrgSpec {
        name: "Praxisgemeinschaft Lindenhof",
        shaped: &[
            "lindenhof-praxis.example",
            "lindenhof-physio.example",
            "lindenhof-termine.example",
        ],
        clean: &[],
        parked: &[],
        recipients: &[],
    },
    OrgSpec {
        name: "Velomanufaktur Fahrwerk",
        shaped: &[
            "fahrwerk-velo.example",
            "fahrwerk-service.example",
            "fahrwerk-outlet.example",
        ],
        clean: &[],
        parked: &[],
        recipients: &[],
    },
    OrgSpec {
        name: OPERATOR,
        shaped: &[],
        clean: &[OPERATOR_DOMAIN, "example-it-support.example"],
        parked: &[],
        recipients: &[],
    },
];

/// Domains that showed up in reports but are not a client's (removed from
/// Marcan): stored as ignored, in the first organization.
pub const IGNORED: [&str; 2] = ["partnerportal-extern.example", "newsletter-dienst.example"];

/// How a reporter writes its report ids.
#[derive(Clone, Copy)]
pub enum IdStyle {
    /// 20 decimal digits.
    Digits,
    /// 32 lowercase hex digits.
    Hex,
    /// `<begin>.<digits>`.
    Stamped,
    /// A UUID.
    Uuid,
}

/// Who sends aggregate reports: `org_name`, the report's `email`, its ids.
#[derive(Clone, Copy)]
pub struct Reporter {
    pub org: &'static str,
    pub email: &'static str,
    pub id: IdStyle,
}

/// The generic receivers, with the `org_name` a baseline matches them by
/// (lowercase). Any other baseline reporter is folded into [`TAIL_REPORTERS`].
pub const GENERIC_REPORTERS: [Reporter; 9] = [
    Reporter {
        org: "Enterprise Outlook",
        email: "dmarcreport@microsoft.com",
        id: IdStyle::Hex,
    },
    Reporter {
        org: "google.com",
        email: "noreply-dmarc-support@google.com",
        id: IdStyle::Digits,
    },
    Reporter {
        org: "Outlook.com",
        email: "dmarcreport@microsoft.com",
        id: IdStyle::Hex,
    },
    Reporter {
        org: "Mimecast",
        email: "dmarc-reports@mimecast.org",
        id: IdStyle::Uuid,
    },
    Reporter {
        org: "GMX",
        email: "dmarc_agg@gmx.net",
        id: IdStyle::Uuid,
    },
    Reporter {
        org: "Yahoo",
        email: "dmarchelp@yahooinc.com",
        id: IdStyle::Stamped,
    },
    Reporter {
        org: "WEB.DE",
        email: "dmarc_agg@web.de",
        id: IdStyle::Uuid,
    },
    Reporter {
        org: "AMAZON-SES",
        email: "postmaster@amazonses.com",
        id: IdStyle::Uuid,
    },
    Reporter {
        org: "zohomail.com",
        email: "noreply-dmarc@zohomail.com",
        id: IdStyle::Stamped,
    },
];

/// Smaller receivers: invented providers and companies, largest first.
pub const TAIL_REPORTERS: [Reporter; 8] = [
    Reporter {
        org: "Talwerk Hosting",
        email: "dmarc@talwerk-hosting.example",
        id: IdStyle::Uuid,
    },
    Reporter {
        org: "seenetz.example",
        email: "noreply-dmarc@seenetz.example",
        id: IdStyle::Digits,
    },
    Reporter {
        org: "Gruenhost",
        email: "dmarc-reports@gruenhost.example",
        id: IdStyle::Uuid,
    },
    Reporter {
        org: "mail.hafenstadt.example",
        email: "postmaster@hafenstadt.example",
        id: IdStyle::Stamped,
    },
    Reporter {
        org: "bergnetz.example",
        email: "dmarc@bergnetz.example",
        id: IdStyle::Hex,
    },
    Reporter {
        org: "relay.weissbach.example",
        email: "dmarc-noreply@weissbach.example",
        id: IdStyle::Uuid,
    },
    Reporter {
        org: "stadtwerke-flussau.example",
        email: "dmarc@stadtwerke-flussau.example",
        id: IdStyle::Digits,
    },
    Reporter {
        org: "praxisnetz.example",
        email: "reports@praxisnetz.example",
        id: IdStyle::Uuid,
    },
];

/// How the tail's share is split among [`TAIL_REPORTERS`].
pub const TAIL_SPLIT: [f64; 8] = [0.40, 0.16, 0.12, 0.09, 0.08, 0.06, 0.05, 0.04];
