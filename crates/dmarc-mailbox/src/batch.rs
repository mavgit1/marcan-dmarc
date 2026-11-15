//! Pure planning for a fetch pass: splitting the new messages into download
//! batches, and the size cap.
//!
//! Kept free of I/O so the rules that decide what is downloaded when (and what
//! is never downloaded) can be unit-tested directly.

use std::time::Duration;

/// Tunables for one fetch pass. [`Limits::DEFAULT`] is what production uses;
/// tests shrink them to exercise batching and timeouts with tiny mailboxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// At most this many message bodies per download (IMAP: `UID FETCH`).
    pub batch_max_messages: usize,
    /// Close a batch before its announced sizes would exceed this many bytes.
    /// A single message larger than this (but under the cap) gets a batch of its own.
    pub batch_max_bytes: u64,
    /// Messages larger than this are quarantined without being downloaded.
    pub max_message_bytes: u64,
    /// Store a batch's parsed messages early once their reports hold this
    /// much XML. The batch limits count compressed bytes, and a parsed
    /// report holds several times its XML in memory, so without this a
    /// batch of small gzip bombs (each under the per-report limit) would
    /// hold gigabytes before it is stored.
    pub max_pending_xml_bytes: u64,
    /// UIDs per `UID FETCH (UID RFC822.SIZE)` command while scanning (IMAP).
    pub size_chunk: usize,
    /// Upper bound for connect + login, EXAMINE, SEARCH, each size chunk and LOGOUT.
    pub command_timeout: Duration,
    /// Base allowance for one body batch (download and ingest)...
    pub batch_timeout_base: Duration,
    /// ...plus one second per this many announced bytes, so a batch of large
    /// messages on a slow link is not cut off while it is still moving.
    pub batch_bytes_per_extra_sec: u64,
}

const MIB: u64 = 1024 * 1024;

impl Limits {
    pub const DEFAULT: Self = Self {
        batch_max_messages: 50,
        batch_max_bytes: 20 * MIB,
        max_message_bytes: 25 * MIB,
        max_pending_xml_bytes: 16 * MIB,
        size_chunk: 500,
        command_timeout: Duration::from_secs(60),
        batch_timeout_base: Duration::from_secs(120),
        batch_bytes_per_extra_sec: 256 * 1024,
    };

    /// Time allowed for a batch whose announced sizes add up to `bytes`.
    pub fn batch_timeout(&self, bytes: u64) -> Duration {
        let extra = bytes / self.batch_bytes_per_extra_sec.max(1);
        self.batch_timeout_base + Duration::from_secs(extra)
    }
}

/// One step of the import: bodies to download and oversized messages to
/// quarantine, by message id `I`. After it completes, every message listed
/// up to `last` has been handled, so the cursor may move past `last`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Batch<I> {
    /// `(id, announced size)` of the messages to download, in import order.
    pub download: Vec<(I, u64)>,
    /// `(id, announced size)` of messages over [`Limits::max_message_bytes`].
    pub too_large: Vec<(I, u64)>,
    /// Sum of the announced sizes in `download`.
    pub bytes: u64,
    /// The batch's last message in import order (downloaded or quarantined).
    pub last: I,
}

impl<I: Clone> Batch<I> {
    /// A batch with nothing in it yet but its last message's id.
    fn empty(last: I) -> Self {
        Self {
            download: Vec::new(),
            too_large: Vec::new(),
            bytes: 0,
            last,
        }
    }

    /// A batch of just one message to download.
    pub fn single((id, size): (I, u64)) -> Self {
        Self {
            download: vec![(id.clone(), size)],
            too_large: Vec::new(),
            bytes: size,
            last: id,
        }
    }

    pub fn download_ids(&self) -> Vec<I> {
        self.download.iter().map(|(id, _)| id.clone()).collect()
    }
}

/// Split `messages` (`(id, announced size)`, in import order) into batches
/// of at most `batch_max_messages` bodies / `batch_max_bytes` announced
/// bytes.
///
/// A message without an announced size is downloaded and counted as 0 bytes
/// (the server did not say; the batch timeout still bounds it). Oversized
/// messages ride along with the batch they fall into so the cursor never
/// passes them before they are quarantined.
pub(crate) fn plan_batches<I: Clone>(
    messages: &[(I, Option<u64>)],
    limits: &Limits,
) -> Vec<Batch<I>> {
    let mut batches = Vec::new();
    let mut current: Option<Batch<I>> = None;
    for (id, size) in messages {
        if let Some(size) = size.filter(|&s| exceeds_size_cap(s, limits)) {
            let batch = current.get_or_insert_with(|| Batch::empty(id.clone()));
            batch.too_large.push((id.clone(), size));
            batch.last = id.clone();
            continue;
        }
        let size = size.unwrap_or(0);
        let full = |b: &mut Batch<I>| {
            !b.download.is_empty()
                && (b.download.len() >= limits.batch_max_messages.max(1)
                    || b.bytes.saturating_add(size) > limits.batch_max_bytes)
        };
        if let Some(done) = current.take_if(full) {
            batches.push(done);
        }
        let batch = current.get_or_insert_with(|| Batch::empty(id.clone()));
        batch.download.push((id.clone(), size));
        batch.bytes += size;
        batch.last = id.clone();
    }
    batches.extend(current);
    batches
}

/// Whether a message of `size` bytes is quarantined instead of downloaded.
pub(crate) fn exceeds_size_cap(size: u64, limits: &Limits) -> bool {
    size > limits.max_message_bytes
}

/// Quarantine reason for a message over the size cap.
pub(crate) fn too_large_reason(size: u64, limits: &Limits) -> String {
    format!(
        "message is {} and exceeds the {} import limit; it was not downloaded",
        format_bytes(size),
        format_bytes(limits.max_message_bytes)
    )
}

/// `"12.3 MB"` style (binary megabytes, one decimal), `"512 KB"` below 1 MB.
pub(crate) fn format_bytes(bytes: u64) -> String {
    if bytes >= MIB {
        format!("{:.1} MB", bytes as f64 / MIB as f64)
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(max_messages: usize, max_bytes: u64, cap: u64) -> Limits {
        Limits {
            batch_max_messages: max_messages,
            batch_max_bytes: max_bytes,
            max_message_bytes: cap,
            ..Limits::DEFAULT
        }
    }

    fn uids(batches: &[Batch<u32>]) -> Vec<Vec<u32>> {
        batches.iter().map(Batch::download_ids).collect()
    }

    #[test]
    fn batches_split_by_count() {
        let msgs: Vec<(u32, Option<u64>)> = (1..=7).map(|u| (u, Some(10))).collect();
        let batches = plan_batches(&msgs, &limits(3, 1_000, 1_000));
        assert_eq!(uids(&batches), vec![vec![1, 2, 3], vec![4, 5, 6], vec![7]]);
        assert_eq!(
            batches.iter().map(|b| b.last).collect::<Vec<_>>(),
            vec![3, 6, 7]
        );
        assert_eq!(batches[0].bytes, 30);
    }

    #[test]
    fn batches_split_by_bytes_and_big_message_goes_alone() {
        let msgs = [
            (1, Some(40)),
            (2, Some(40)),
            (3, Some(40)),  // 120 > 100: new batch
            (4, Some(150)), // bigger than a batch, under the cap: alone
            (5, Some(10)),
        ];
        let batches = plan_batches(&msgs, &limits(50, 100, 1_000));
        assert_eq!(uids(&batches), vec![vec![1, 2], vec![3], vec![4], vec![5]]);
        assert_eq!(batches[2].bytes, 150);
    }

    #[test]
    fn oversized_messages_are_quarantined_in_place() {
        let msgs = [
            (10, Some(5)),
            (11, Some(5_000)),
            (12, Some(5)),
            (13, Some(5)),
            (14, Some(9_999)),
        ];
        let batches = plan_batches(&msgs, &limits(2, 1_000, 1_000));
        assert_eq!(uids(&batches), vec![vec![10, 12], vec![13]]);
        assert_eq!(batches[0].too_large, vec![(11, 5_000)]);
        assert_eq!(batches[0].last, 12);
        // Trailing oversize message stays with the last batch, so the cursor passes it.
        assert_eq!(batches[1].too_large, vec![(14, 9_999)]);
        assert_eq!(batches[1].last, 14);
    }

    #[test]
    fn only_oversized_still_yields_a_batch() {
        let batches = plan_batches(&[(3, Some(2_000))], &limits(2, 1_000, 1_000));
        assert_eq!(batches.len(), 1);
        assert!(batches[0].download.is_empty());
        assert_eq!(batches[0].last, 3);
    }

    #[test]
    fn unknown_size_is_downloaded() {
        let batches = plan_batches(&[(1, None), (2, None)], &limits(50, 10, 10));
        assert_eq!(uids(&batches), vec![vec![1, 2]]);
        assert_eq!(batches[0].bytes, 0);
    }

    #[test]
    fn size_cap_boundary() {
        let l = limits(50, 100, 100);
        assert!(!exceeds_size_cap(100, &l));
        assert!(exceeds_size_cap(101, &l));
        assert!(!exceeds_size_cap(25 * MIB, &Limits::DEFAULT));
        assert!(exceeds_size_cap(25 * MIB + 1, &Limits::DEFAULT));
        let reason = too_large_reason(30 * MIB, &Limits::DEFAULT);
        assert!(reason.contains("30.0 MB"), "{reason}");
        assert!(reason.contains("25.0 MB"), "{reason}");
        assert_eq!(format_bytes(1), "1 KB");
    }

    #[test]
    fn empty_plan() {
        assert!(plan_batches::<u32>(&[], &Limits::DEFAULT).is_empty());
    }

    #[test]
    fn batch_timeout_scales_with_bytes() {
        let l = Limits::DEFAULT;
        assert_eq!(l.batch_timeout(0), Duration::from_secs(120));
        assert_eq!(l.batch_timeout(20 * MIB), Duration::from_secs(120 + 80));
    }

    #[test]
    fn three_thousand_messages_make_sixty_batches() {
        let msgs: Vec<(u32, Option<u64>)> = (1..=3_000).map(|u| (u, Some(30_000))).collect();
        let batches = plan_batches(&msgs, &Limits::DEFAULT);
        assert_eq!(batches.len(), 60);
        assert!(batches.iter().all(|b| b.download.len() == 50));
        assert_eq!(batches.last().unwrap().last, 3_000);
    }
}
