//! Results of slow lookups kept in the database, and the background work that
//! keeps them fresh: DNS checks per domain and reverse-DNS evidence per sender
//! IP. Handlers read these and may start a refresh; they never wait on one.
//! The loops themselves are driven by [`crate::Background`].

pub mod dns;
pub mod identity;
