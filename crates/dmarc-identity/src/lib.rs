//! Sender identity for DMARC aggregate reports.
//!
//! Reports name a sender only by its IP. Marcan names it from evidence, never
//! from a hand-kept list of netblocks:
//!
//! - reverse DNS, reduced to an operator domain ([`operator_domain`]);
//! - SPF: a domain whose SPF passed for the IP in reports.
//!
//! [`resolve`] combines them; [`SenderNames`] turns the IP and its operator
//! domain into a display name (the vendored parsedmarc map plus optional user
//! overrides by IP, range or domain).
//! An identity is a caption only and never feeds a verdict, colour or sort.
//!
//! This crate performs no DNS: callers pass already-resolved evidence.

mod ip;
mod names;
mod operator;
mod resolve;

pub use ip::{classify_ip, IpKind};
pub use names::{
    format_overrides, parse_overrides, parse_target, NameOverride, OverrideLineError, SenderNames,
};
pub use operator::operator_domain;
pub use resolve::{resolve, Evidence, IdentitySource, SenderIdentity};
