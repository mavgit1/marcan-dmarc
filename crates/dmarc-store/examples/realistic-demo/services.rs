//! Who sends the mail: the mail platforms, gateways and mailing services, each
//! with its addresses (documentation ranges only: 192.0.2.0/24,
//! 198.51.100.0/24, 203.0.113.0/24, `2001:db8::/32`), the reverse DNS those
//! addresses are given (stored as identity snapshots: the server never looks
//! a documentation address up, so they stay) and the SPF record that
//! authorizes them in the demo's DNS.

use crate::rng::Rng;
use std::collections::BTreeMap;

/// One sending address and its reverse DNS: `(hostname, forward-confirmed)`.
#[derive(Clone)]
pub struct Host {
    pub ip: String,
    pub ptr: Option<(String, bool)>,
}

/// A mailing service a domain sends newsletters or notifications through.
pub struct Esp {
    /// Its name in Marcan's bundled sender-name map.
    pub name: &'static str,
    /// The SPF include a domain adds for it.
    pub include: &'static str,
    /// Its own SPF record (made up, covering only its demo addresses).
    pub spf: &'static str,
    /// The envelope sender (bounce) domain: SPF passes for it, not aligned.
    pub envelope: &'static str,
    /// The domain of its own DKIM signature beside the customer's.
    pub signer: &'static str,
    /// The selector a customer publishes for it.
    pub selector: &'static str,
    pub hosts: Vec<Host>,
}

/// A host or gateway that relays mail on (forwarding breaks SPF and, when
/// the body is rewritten, DKIM).
pub struct Forwarder {
    pub hosts: Vec<Host>,
}

/// A small hosting provider some domains run their mail on (invented).
pub struct Hoster {
    pub include: &'static str,
    pub spf: String,
    pub selector: &'static str,
    pub hosts: Vec<Host>,
}

pub struct Services {
    pub microsoft: Vec<Host>,
    pub google: Vec<Host>,
    pub esps: Vec<Esp>,
    pub forwarders: Vec<Forwarder>,
    pub hosters: Vec<Hoster>,
    /// Addresses spoofed mail comes from: mostly no reverse DNS.
    pub spoofers: Vec<Host>,
}

pub const MICROSOFT_INCLUDE: &str = "spf.protection.outlook.com";
pub const GOOGLE_INCLUDE: &str = "_spf.google.com";

fn v4(net: &str, last: u32) -> String {
    format!("{net}.{last}")
}

fn hosts(ips: Vec<String>, mut ptr: impl FnMut(usize) -> Option<(String, bool)>) -> Vec<Host> {
    ips.into_iter()
        .enumerate()
        .map(|(i, ip)| Host { ip, ptr: ptr(i) })
        .collect()
}

fn range(net: &str, first: u32, count: u32) -> Vec<String> {
    (first..first + count).map(|n| v4(net, n)).collect()
}

const DOC1: &str = "192.0.2";
const DOC2: &str = "198.51.100";
const DOC3: &str = "203.0.113";

impl Services {
    pub fn new(rng: &mut Rng) -> Self {
        // Microsoft 365: the whole of 198.51.100.0/24 and an IPv6 block.
        let regions = [
            "db8eur05on",
            "am7eur03on",
            "fr2deu01on",
            "vi1eur02on",
            "pa4eur04on",
        ];
        let mut microsoft_ips = range(DOC2, 1, 254);
        microsoft_ips
            .extend((0..300).map(|n| format!("2001:db8:f400:7e{:02x}::{:x}", n / 40, 0x100 + n)));
        let microsoft = hosts(microsoft_ips, |i| {
            Some((
                format!(
                    "mail-{}{}.outbound.protection.outlook.com",
                    regions[i % regions.len()],
                    2050 + (i * 7) % 900
                ),
                true,
            ))
        });

        // Google Workspace: 192.0.2.16/28 and an IPv6 block.
        let mut google_ips = range(DOC1, 16, 16);
        google_ips.extend((1..=40).map(|n| format!("2001:db8:4b0:4864:20::{n:x}")));
        let google_hosts = ["wm1", "ej1", "lf1", "wr1", "qt1", "yb1", "pj1"];
        let google = hosts(google_ips, |i| {
            Some((
                format!(
                    "mail-{}-f{}.google.com",
                    google_hosts[i % google_hosts.len()],
                    41 + (i * 13) % 150
                ),
                true,
            ))
        });

        let esp = |name,
                   include,
                   spf,
                   envelope,
                   signer,
                   selector,
                   first,
                   count,
                   ptr: fn(usize) -> String| Esp {
            name,
            include,
            spf,
            envelope,
            signer,
            selector,
            hosts: hosts(range(DOC1, first, count), |i| Some((ptr(i), true))),
        };
        let esps = vec![
            esp(
                "Amazon SES",
                "amazonses.com",
                "v=spf1 ip4:192.0.2.64/28 -all",
                "eu-west-1.amazonses.com",
                "amazonses.com",
                "ses2025",
                64,
                16,
                |i| {
                    format!(
                        "a{}-{}.smtp-out.eu-west-1.amazonses.com",
                        7 + i % 3,
                        40 + i * 3
                    )
                },
            ),
            esp(
                "Twilio SendGrid",
                "sendgrid.net",
                "v=spf1 ip4:192.0.2.80/29 -all",
                "sendgrid.net",
                "sendgrid.info",
                "s1",
                80,
                8,
                |i| format!("o{}.ptr{}.sendgrid.net", 1 + i, 2100 + i * 17),
            ),
            esp(
                "Intuit Mailchimp",
                "servers.mcsv.net",
                "v=spf1 ip4:192.0.2.88/29 -all",
                "mail158.suw131.mcsv.net",
                "mcsv.net",
                "k1",
                88,
                8,
                |i| format!("mail{}.suw{}.mcsv.net", 150 + i, 120 + i % 5),
            ),
            esp(
                "Mailchimp Transactional",
                "spf.mandrillapp.com",
                "v=spf1 ip4:192.0.2.96/29 -all",
                "mandrillapp.com",
                "mandrillapp.com",
                "mte1",
                96,
                8,
                |i| format!("mail{}.mandrillapp.com", 128 + i * 3),
            ),
            esp(
                "HubSpot",
                "spf.hubspotemail.net",
                "v=spf1 ip4:192.0.2.104/29 -all",
                "bf04x.hubspotemail.net",
                "hubspotemail.net",
                "hs1",
                104,
                8,
                |i| format!("{}.hubspotemail.net", 10 + i),
            ),
            esp(
                "Zoho",
                "zoho.eu",
                "v=spf1 ip4:192.0.2.112/29 -all",
                "bounce.zohomail.com",
                "zoho.eu",
                "zmail",
                112,
                8,
                |i| format!("sender{}-{}.zoho.com", 4 + i % 3, 200 + i),
            ),
            esp(
                "Salesforce",
                "_spf.salesforce.com",
                "v=spf1 ip4:192.0.2.120/29 -all",
                "bnc3.mail.salesforce.com",
                "salesforce.com",
                "sf2024",
                120,
                8,
                |i| format!("smtp{}-ia{}.mta.salesforce.com", 10 + i, 4 + i % 2),
            ),
        ];

        // Gateways and forwarders: the security services mail passes through
        // on its way, and Google's forwarding.
        let forwarder = |first, count, ptr: fn(usize) -> String| Forwarder {
            hosts: hosts(range(DOC1, first, count), |i| Some((ptr(i), true))),
        };
        let mut forwarders = vec![
            forwarder(32, 8, |i| {
                format!("eu-smtp-delivery-{}.mimecast.com", 100 + i * 11)
            }),
            forwarder(40, 4, |i| format!("mail{}.tmcas.trendmicro.com", 30 + i)),
            forwarder(44, 4, |i| {
                format!("mx0{}-00{}.pphosted.com", ["a", "b"][i % 2], 1_900 + i)
            }),
            forwarder(48, 4, |i| format!("out{}.cloud-sec-av.com", 20 + i)),
            forwarder(52, 4, |i| format!("smtp{}.eu.inkyphishfence.com", 2 + i)),
            forwarder(56, 4, |i| {
                format!("esa{}.hc3370-{}.eu.iphmx.com", 1 + i % 2, 44 + i)
            }),
        ];
        forwarders.push(Forwarder {
            hosts: hosts(
                (1..=12)
                    .map(|n| format!("2001:db8:4b0:4860::{n:x}"))
                    .collect(),
                |i| {
                    Some((
                        format!("mail-sor-f{}.unverified-forwarding.1e100.net", 60 + i),
                        true,
                    ))
                },
            ),
        });

        let hoster = |name: &'static str,
                      include: &'static str,
                      net_first: u32,
                      selector,
                      prefix: &'static str| {
            Hoster {
                include,
                spf: format!("v=spf1 ip4:{DOC1}.{net_first}/28 -all"),
                selector,
                hosts: hosts(range(DOC1, net_first, 16), |i| {
                    Some((format!("{prefix}{}.{name}", 1 + i), true))
                }),
            }
        };
        let hosters = vec![
            hoster(
                "talwerk-hosting.example",
                "spf.talwerk-hosting.example",
                128,
                "default",
                "mx",
            ),
            hoster(
                "seenetz.example",
                "_spf.seenetz.example",
                144,
                "mail",
                "smtp",
            ),
            hoster(
                "gruenhost.example",
                "spf.gruenhost.example",
                160,
                "gh1",
                "out",
            ),
        ];

        // Spoofers: anywhere in 203.0.113.0/24 and a v6 block; a quarter
        // with a residential or rented-server name (invented, unconfirmed).
        let mut spoof_ips = range(DOC3, 1, 254);
        spoof_ips.extend((1..=40).map(|n| format!("2001:db8:7a00:{:x}::{:x}", n % 7, 0x30 + n)));
        let spoofers = hosts(spoof_ips, |i| {
            if !rng.chance(0.25) {
                return None;
            }
            let host = if i % 2 == 0 {
                format!("dyn-{}-{}.kabelnetz-ost.example", 40 + i % 200, i % 251)
            } else {
                format!("vps{}.billigserver.example", 1_000 + i * 7)
            };
            Some((host, rng.chance(0.3)))
        });

        Self {
            microsoft,
            google,
            esps,
            forwarders,
            hosters,
            spoofers,
        }
    }

    /// The made-up SPF records of every include the demo's domains use.
    pub fn spf_records(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        out.insert(
            MICROSOFT_INCLUDE.to_owned(),
            "v=spf1 ip4:198.51.100.0/24 ip6:2001:db8:f400::/48 -all".to_owned(),
        );
        out.insert(
            GOOGLE_INCLUDE.to_owned(),
            "v=spf1 ip4:192.0.2.16/28 ip6:2001:db8:4b0::/48 ~all".to_owned(),
        );
        for esp in &self.esps {
            out.insert(esp.include.to_owned(), esp.spf.to_owned());
        }
        for hoster in &self.hosters {
            out.insert(hoster.include.to_owned(), hoster.spf.clone());
        }
        out
    }

    /// Every address with its reverse DNS.
    pub fn all_hosts(&self) -> impl Iterator<Item = &Host> {
        self.microsoft
            .iter()
            .chain(&self.google)
            .chain(self.esps.iter().flat_map(|e| &e.hosts))
            .chain(self.forwarders.iter().flat_map(|f| &f.hosts))
            .chain(self.hosters.iter().flat_map(|h| &h.hosts))
            .chain(&self.spoofers)
    }
}
