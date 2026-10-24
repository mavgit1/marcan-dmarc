//! What ingest does with a parsed report next: RFC 7489 alignment of every
//! auth result against the record's From domain, in both modes.

use dmarc_parser::{
    dkim_auth_aligned, spf_auth_aligned, spf_counts_for_dmarc, AlignmentMode, Report,
};

pub fn align_all(report: &Report) {
    let modes = [
        AlignmentMode::from_tag(report.policy.adkim.as_deref()),
        AlignmentMode::from_tag(report.policy.aspf.as_deref()),
        AlignmentMode::Relaxed,
        AlignmentMode::Strict,
    ];
    for record in &report.records {
        let from = record.header_from.as_deref();
        for mode in modes {
            for dkim in &record.dkim_auth {
                std::hint::black_box(dkim_auth_aligned(dkim, from, mode));
            }
            for spf in &record.spf_auth {
                std::hint::black_box(spf_auth_aligned(spf, from, mode));
                std::hint::black_box(spf_counts_for_dmarc(spf, &record.spf_auth));
            }
        }
    }
}
