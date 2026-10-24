//! The demo's DNS: the TXT records each domain publishes, made to agree with
//! its reports and to earn its class's grade from the real checker (the
//! server answers every TXT lookup from these while they are stored).

use crate::catalogue::{OPERATOR_DOMAIN, REPORT_ADDRESS};
use crate::profile::DnsClass;
use crate::services::{Services, GOOGLE_INCLUDE, MICROSOFT_INCLUDE};
use crate::traffic::{Plan, Platform, Role};
use std::collections::BTreeMap;

/// A 2048-bit RSA public key (SPKI, Base64), made for the demo.
const KEY_2048: &str = "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA4Qe0CpUby/aK3HZKigFMjyMEaAyfPtYhzV+RxsvT8g9CGY0YaovG0Tz5xutPzVrIbvySrncyrV0MOTmJyf12ZLtKW9RsKmw+uV5xq476f6mLItAw0I/++qbGEwjCTVVXvX8ROJV35pO+afp6q2S1v/Tze1eOMlD8qkYt64Q676ljtF3TPqRbop87YAUoh8JnJISq6bccdZ6Dj9wnv9vEAs1OvYB7xOf8tdMdrxyhd5H3cO7iSOgopiJZ4sW+xZPBYcjOf8aLxLeaxplxoNqBam9fkvF7f2lSG4RAi60Bc+R/VIo90ooMDGcxiIwcwT0VmnpWBQhf+HqOnO0PPcnJ6QIDAQAB";

/// A 1024-bit RSA public key (SPKI, Base64), made for the demo: "dated".
const KEY_1024: &str = "MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQCrfbTpP3IpU56kUpkRkZ5mdcv8XANOoffUs1ATFczfngZ+n/SYi1d3w0r+HdSjuB/niIFLSMMQi49+YPxsb7q5Z2dwO2J3Z4Pb24/ywcE5ovMU21lHoxOmL6RXa1uyGPmeGsCSnVBweiR15723B3qYtJXEAJESLGCI1tw2Wb6sVQIDAQAB";

pub fn records(services: &Services, plans: &[Plan]) -> BTreeMap<String, Vec<String>> {
    let mut table: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut put = |name: String, record: String| table.entry(name).or_default().push(record);

    for (name, spf) in services.spf_records() {
        put(name, spf);
    }
    // The report address accepts every domain's reports (RFC 7489 §7.1).
    put(
        format!("*._report._dmarc.{OPERATOR_DOMAIN}"),
        "v=DMARC1".into(),
    );

    let all_includes: Vec<&str> = [MICROSOFT_INCLUDE, GOOGLE_INCLUDE]
        .into_iter()
        .chain(services.esps.iter().map(|e| e.include))
        .chain(services.hosters.iter().map(|h| h.include))
        .collect();

    for plan in plans {
        let d = plan.name;
        match plan.role {
            Role::Ignored => continue,
            Role::Parked => {
                put(
                    format!("_dmarc.{d}"),
                    format!("v=DMARC1; p=reject; sp=reject; adkim=s; aspf=s; rua=mailto:{REPORT_ADDRESS}"),
                );
                put(d.to_owned(), "v=spf1 -all".into());
                put(format!("*._domainkey.{d}"), "v=DKIM1; p=".into());
                continue;
            }
            Role::Shaped | Role::Clean => {}
        }
        let class = plan.shape.dns;
        let rua = format!("rua=mailto:{REPORT_ADDRESS}");
        let dmarc = match class {
            DnsClass::Clean => format!("v=DMARC1; p=reject; sp=reject; {rua}; fo=1"),
            DnsClass::RejectNoRua => "v=DMARC1; p=reject".into(),
            DnsClass::Quarantine => format!("v=DMARC1; p=quarantine; pct=100; {rua}"),
            p if p.policy() == "reject" => format!("v=DMARC1; p=reject; {rua}"),
            _ => format!("v=DMARC1; p=none; {rua}"),
        };
        put(format!("_dmarc.{d}"), dmarc);
        if class == DnsClass::Clean {
            put(
                format!("_smtp._tls.{d}"),
                format!("v=TLSRPTv1; rua=mailto:tls-reports@{OPERATOR_DOMAIN}"),
            );
        }

        // SPF: the platform and the signing services, padded for the
        // classes judged on their lookups.
        let mut includes: Vec<&str> = vec![match plan.platform {
            Platform::Microsoft => MICROSOFT_INCLUDE,
            Platform::Google => GOOGLE_INCLUDE,
            Platform::Hoster(h) => services.hosters[h].include,
        }];
        includes.extend(plan.esps.iter().map(|(e, _)| services.esps[*e].include));
        let want = match class {
            DnsClass::Quarantine => 8,
            DnsClass::NoneOverLimit => 11,
            _ => 0,
        };
        for include in &all_includes {
            if includes.len() >= want {
                break;
            }
            if !includes.contains(include) {
                includes.push(include);
            }
        }
        let all = match class {
            DnsClass::RejectSoftfail | DnsClass::NoneOverLimit => "~all",
            DnsClass::NoneNeutral => "?all",
            _ => "-all",
        };
        let mechanisms: Vec<String> = includes.iter().map(|i| format!("include:{i}")).collect();
        // A long list moves into an included record of the domain's own,
        // as people do to keep a TXT string under 255 characters.
        let (head, tail) = mechanisms.split_at(mechanisms.len().min(5));
        let mut spf = format!("v=spf1 {}", head.join(" "));
        if !tail.is_empty() {
            put(
                format!("_spf.{d}"),
                format!("v=spf1 {} {all}", tail.join(" ")),
            );
            spf.push_str(&format!(" include:_spf.{d}"));
        }
        put(d.to_owned(), format!("{spf} {all}"));

        // DKIM: the platform's selectors and each signing service's.
        for (i, selector) in plan.selectors.iter().enumerate() {
            let last = i + 1 == plan.selectors.len();
            if class == DnsClass::NoneMissingSelector && last {
                continue;
            }
            let dated = i == 0 && matches!(class, DnsClass::NoneDated | DnsClass::Quarantine);
            let key = if dated { KEY_1024 } else { KEY_2048 };
            put(
                format!("{selector}._domainkey.{d}"),
                format!("v=DKIM1; k=rsa; p={key}"),
            );
        }
        for (e, _) in &plan.esps {
            put(
                format!("{}._domainkey.{d}", services.esps[*e].selector),
                format!("v=DKIM1; k=rsa; p={KEY_2048}"),
            );
        }
    }
    table
}
