//! Bounded journal for HID traffic observed at the application's `Transport` boundary.
//!
//! Sequence and timestamps describe transport-call completion order. They are not physical USB
//! bus chronology. RX lengths and bytes are those returned by the selected transport after any
//! HID normalization; bytes rejected before that boundary cannot be retained here.

use std::collections::VecDeque;
use std::fmt::{self, Write as _};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::transport::Transport;

pub const TRAFFIC_EVENT_COUNT_LIMIT: usize = 1_024;
pub const TRAFFIC_RETAINED_PAYLOAD_BYTES_LIMIT: usize = 1024 * 1024;
pub const TRAFFIC_EVENT_PAYLOAD_BYTES_LIMIT: usize = 4_096;
pub const TRAFFIC_ERROR_TEXT_BYTES_LIMIT: usize = 256;
pub const TRAFFIC_WINDOW_EVENT_LIMIT: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TrafficSequence(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficDirection {
    Rx,
    Tx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficOutcome {
    ReadReturned,
    ReadFailed,
    WriteSucceeded,
    WriteFailedDeliveryUncertain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodedEventKind {
    Snapshot,
    QueryReply,
    Meter,
    Auxiliary,
    Notification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeStatus {
    Pending,
    Accepted(DecodedEventKind),
    /// The driver deliberately returned no event. No reason is inferred by the journal.
    Ignored,
    Rejected {
        error: Arc<str>,
        truncated: bool,
    },
}

/// Numeric envelope fields only. Fields with protocol-specific positions are populated only for
/// a structurally matching report prefix; no endpoint or semantic meaning is inferred.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrafficClassifier {
    pub byte_0: Option<u8>,
    pub byte_1: Option<u8>,
    pub command_opcode_at_4: Option<u8>,
    pub query_category_at_8: Option<u8>,
    pub query_index_at_12: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrafficEvent {
    pub sequence: TrafficSequence,
    pub completed_after: Duration,
    pub direction: TrafficDirection,
    pub outcome: TrafficOutcome,
    /// Bytes returned at this application boundary, or bytes attempted for a write.
    pub reported_len: usize,
    pub retained_bytes: Arc<[u8]>,
    pub payload_truncated: bool,
    pub error: Option<Arc<str>>,
    pub error_truncated: bool,
    pub classifier: TrafficClassifier,
    pub decode_status: Option<DecodeStatus>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrafficCounters {
    pub rx_returned: u64,
    pub rx_errors: u64,
    pub read_timeouts: u64,
    pub tx_succeeded: u64,
    pub tx_failed_delivery_uncertain: u64,
    pub evicted_by_count: u64,
    pub evicted_by_bytes: u64,
    pub payload_truncations: u64,
    pub error_text_truncations: u64,
    pub decoder_accepted: u64,
    pub decoder_ignored: u64,
    pub decoder_rejected: u64,
    pub decode_updates_lost: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrafficJournalStats {
    pub counters: TrafficCounters,
    pub retained_events: usize,
    pub retained_payload_bytes: usize,
}

#[derive(Clone)]
pub struct TrafficJournal {
    inner: Arc<Mutex<TrafficJournalInner>>,
}

struct TrafficJournalInner {
    started_at: Instant,
    next_sequence: u64,
    retained_payload_bytes: usize,
    events: VecDeque<TrafficEvent>,
    counters: TrafficCounters,
}

impl Default for TrafficJournal {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TrafficJournalInner {
                started_at: Instant::now(),
                next_sequence: 1,
                retained_payload_bytes: 0,
                events: VecDeque::new(),
                counters: TrafficCounters::default(),
            })),
        }
    }
}

impl TrafficJournal {
    pub fn stats(&self) -> TrafficJournalStats {
        let inner = self.lock();
        TrafficJournalStats {
            counters: inner.counters,
            retained_events: inner.events.len(),
            retained_payload_bytes: inner.retained_payload_bytes,
        }
    }

    /// Return an oldest-to-newest, bounded window. Sequence bounds are exclusive.
    ///
    /// `after` pages forward from a cursor. Without `after`, the newest events before the optional
    /// `before` cursor are returned, which supports both an initial tail and backward paging.
    pub fn window(
        &self,
        after: Option<TrafficSequence>,
        before: Option<TrafficSequence>,
        max_events: usize,
    ) -> Vec<TrafficEvent> {
        let max_events = max_events.min(TRAFFIC_WINDOW_EVENT_LIMIT);
        if max_events == 0 {
            return Vec::new();
        }
        let inner = self.lock();
        if let Some(after) = after {
            return inner
                .events
                .iter()
                .filter(|event| event.sequence > after)
                .filter(|event| before.is_none_or(|sequence| event.sequence < sequence))
                .take(max_events)
                .cloned()
                .collect();
        }
        let mut newest = inner
            .events
            .iter()
            .rev()
            .filter(|event| before.is_none_or(|sequence| event.sequence < sequence))
            .take(max_events)
            .cloned()
            .collect::<Vec<_>>();
        newest.reverse();
        newest
    }

    /// Look up one retained event. Cloning shares its payload and error allocations through `Arc`.
    pub fn event(&self, sequence: TrafficSequence) -> Option<TrafficEvent> {
        self.lock()
            .events
            .iter()
            .find(|event| event.sequence == sequence)
            .cloned()
    }

    pub(crate) fn record_read_returned(&self, bytes: &[u8]) -> TrafficSequence {
        let mut inner = self.lock();
        inner.counters.rx_returned = inner.counters.rx_returned.saturating_add(1);
        inner.push_payload_event(
            TrafficDirection::Rx,
            TrafficOutcome::ReadReturned,
            bytes,
            None,
            false,
            Some(DecodeStatus::Pending),
        )
    }

    pub(crate) fn record_read_error(&self, error: &anyhow::Error) {
        let (error, truncated) = bounded_error(error);
        let mut inner = self.lock();
        inner.counters.rx_errors = inner.counters.rx_errors.saturating_add(1);
        if truncated {
            inner.counters.error_text_truncations =
                inner.counters.error_text_truncations.saturating_add(1);
        }
        inner.push_payload_event(
            TrafficDirection::Rx,
            TrafficOutcome::ReadFailed,
            &[],
            Some(error),
            truncated,
            None,
        );
    }

    pub(crate) fn record_timeout(&self) {
        let mut inner = self.lock();
        inner.counters.read_timeouts = inner.counters.read_timeouts.saturating_add(1);
    }

    pub(crate) fn record_write(&self, bytes: &[u8], result: &Result<()>) {
        let error = result.as_ref().err().map(bounded_error);
        let mut inner = self.lock();
        let outcome = if result.is_ok() {
            inner.counters.tx_succeeded = inner.counters.tx_succeeded.saturating_add(1);
            TrafficOutcome::WriteSucceeded
        } else {
            inner.counters.tx_failed_delivery_uncertain = inner
                .counters
                .tx_failed_delivery_uncertain
                .saturating_add(1);
            TrafficOutcome::WriteFailedDeliveryUncertain
        };
        let (error, error_truncated) = error
            .map(|(text, truncated)| (Some(text), truncated))
            .unwrap_or((None, false));
        if error_truncated {
            inner.counters.error_text_truncations =
                inner.counters.error_text_truncations.saturating_add(1);
        }
        inner.push_payload_event(
            TrafficDirection::Tx,
            outcome,
            bytes,
            error,
            error_truncated,
            None,
        );
    }

    pub(crate) fn mark_decode(&self, sequence: TrafficSequence, status: DecodeStatus) {
        let mut inner = self.lock();
        let Some(index) = inner
            .events
            .iter()
            .position(|event| event.sequence == sequence)
        else {
            inner.counters.decode_updates_lost =
                inner.counters.decode_updates_lost.saturating_add(1);
            return;
        };
        if inner.events[index].decode_status != Some(DecodeStatus::Pending) {
            inner.counters.decode_updates_lost =
                inner.counters.decode_updates_lost.saturating_add(1);
            return;
        }
        match &status {
            DecodeStatus::Accepted(_) => {
                inner.counters.decoder_accepted = inner.counters.decoder_accepted.saturating_add(1);
            }
            DecodeStatus::Ignored => {
                inner.counters.decoder_ignored = inner.counters.decoder_ignored.saturating_add(1);
            }
            DecodeStatus::Rejected { truncated, .. } => {
                inner.counters.decoder_rejected = inner.counters.decoder_rejected.saturating_add(1);
                if *truncated {
                    inner.counters.error_text_truncations =
                        inner.counters.error_text_truncations.saturating_add(1);
                }
            }
            DecodeStatus::Pending => {
                inner.counters.decode_updates_lost =
                    inner.counters.decode_updates_lost.saturating_add(1);
                return;
            }
        }
        inner.events[index].decode_status = Some(status);
    }

    pub(crate) fn mark_decode_rejected(&self, sequence: TrafficSequence, error: &dyn fmt::Display) {
        let (error, truncated) = bounded_display(error);
        self.mark_decode(sequence, DecodeStatus::Rejected { error, truncated });
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TrafficJournalInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl TrafficJournalInner {
    fn push_payload_event(
        &mut self,
        direction: TrafficDirection,
        outcome: TrafficOutcome,
        bytes: &[u8],
        error: Option<Arc<str>>,
        error_truncated: bool,
        decode_status: Option<DecodeStatus>,
    ) -> TrafficSequence {
        let retained_len = bytes.len().min(TRAFFIC_EVENT_PAYLOAD_BYTES_LIMIT);
        let payload_truncated = retained_len != bytes.len();
        if payload_truncated {
            self.counters.payload_truncations = self.counters.payload_truncations.saturating_add(1);
        }
        // Only the retained prefix is cloned; the full returned/attempted payload is never copied.
        let retained_bytes: Arc<[u8]> = Arc::from(&bytes[..retained_len]);
        let sequence = TrafficSequence(self.next_sequence);
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.retained_payload_bytes = self
            .retained_payload_bytes
            .saturating_add(retained_bytes.len());
        self.events.push_back(TrafficEvent {
            sequence,
            completed_after: self.started_at.elapsed(),
            direction,
            outcome,
            reported_len: bytes.len(),
            retained_bytes,
            payload_truncated,
            error,
            error_truncated,
            classifier: classify(bytes),
            decode_status,
        });
        self.enforce_bounds();
        sequence
    }

    fn enforce_bounds(&mut self) {
        loop {
            let reason = if self.events.len() > TRAFFIC_EVENT_COUNT_LIMIT {
                Some(EvictionReason::Count)
            } else if self.retained_payload_bytes > TRAFFIC_RETAINED_PAYLOAD_BYTES_LIMIT {
                Some(EvictionReason::Bytes)
            } else {
                None
            };
            let Some(reason) = reason else {
                break;
            };
            let evicted = self
                .events
                .pop_front()
                .expect("a violated traffic bound requires a retained event");
            self.retained_payload_bytes = self
                .retained_payload_bytes
                .checked_sub(evicted.retained_bytes.len())
                .expect("retained traffic byte accounting must not underflow");
            match reason {
                EvictionReason::Count => {
                    self.counters.evicted_by_count =
                        self.counters.evicted_by_count.saturating_add(1);
                }
                EvictionReason::Bytes => {
                    self.counters.evicted_by_bytes =
                        self.counters.evicted_by_bytes.saturating_add(1);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum EvictionReason {
    Count,
    Bytes,
}

fn classify(bytes: &[u8]) -> TrafficClassifier {
    let byte_0 = bytes.first().copied();
    let byte_1 = bytes.get(1).copied();
    let command_opcode_at_4 = (byte_0 == Some(0x70))
        .then(|| bytes.get(4).copied())
        .flatten();
    let query_shape = byte_0 == Some(0x74) || (byte_0 == Some(0x75) && byte_1 == Some(0x00));
    TrafficClassifier {
        byte_0,
        byte_1,
        command_opcode_at_4,
        query_category_at_8: query_shape.then(|| bytes.get(8).copied()).flatten(),
        query_index_at_12: query_shape.then(|| bytes.get(12).copied()).flatten(),
    }
}

struct BoundedText {
    text: String,
    truncated: bool,
}

impl BoundedText {
    fn new() -> Self {
        Self {
            text: String::with_capacity(TRAFFIC_ERROR_TEXT_BYTES_LIMIT),
            truncated: false,
        }
    }
}

impl fmt::Write for BoundedText {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let remaining = TRAFFIC_ERROR_TEXT_BYTES_LIMIT.saturating_sub(self.text.len());
        if value.len() <= remaining {
            self.text.push_str(value);
            return Ok(());
        }
        let mut end = remaining;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        self.text.push_str(&value[..end]);
        self.truncated = true;
        Ok(())
    }
}

fn bounded_error(error: &anyhow::Error) -> (Arc<str>, bool) {
    bounded_display(&format_args!("{error:#}"))
}

fn bounded_display(value: &dyn fmt::Display) -> (Arc<str>, bool) {
    let mut bounded = BoundedText::new();
    let _ = write!(&mut bounded, "{value}");
    (Arc::from(bounded.text), bounded.truncated)
}

/// The sole traffic observation boundary used by `Controller`.
pub(crate) struct ObservedTransport {
    inner: Box<dyn Transport>,
    journal: TrafficJournal,
}

pub(crate) struct ObservedRead {
    pub result: Result<Option<Vec<u8>>>,
    pub sequence: Option<TrafficSequence>,
}

impl ObservedTransport {
    pub fn new(inner: Box<dyn Transport>, journal: TrafficJournal) -> Self {
        Self { inner, journal }
    }

    pub fn read_with_sequence(&self, timeout: Duration) -> ObservedRead {
        let result = self.inner.read(timeout);
        let sequence = match &result {
            Ok(Some(bytes)) => Some(self.journal.record_read_returned(bytes)),
            Ok(None) => {
                self.journal.record_timeout();
                None
            }
            Err(error) => {
                self.journal.record_read_error(error);
                None
            }
        };
        ObservedRead { result, sequence }
    }
}

impl Transport for ObservedTransport {
    fn write(&self, data: &[u8]) -> Result<()> {
        let result = self.inner.write(data);
        self.journal.record_write(data, &result);
        result
    }

    fn read(&self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        self.read_with_sequence(timeout).result
    }

    fn is_available(&self) -> Result<bool> {
        self.inner.is_available()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone, Default)]
    struct ScriptedTransport {
        calls: Arc<AtomicUsize>,
        reads: Arc<Mutex<VecDeque<Result<Option<Vec<u8>>>>>>,
        write_error: Arc<Mutex<Option<String>>>,
    }

    impl Transport for ScriptedTransport {
        fn write(&self, _data: &[u8]) -> Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.write_error.lock().expect("write error").as_ref() {
                Some(error) => Err(anyhow!(error.clone())),
                None => Ok(()),
            }
        }

        fn read(&self, _timeout: Duration) -> Result<Option<Vec<u8>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.reads
                .lock()
                .expect("reads")
                .pop_front()
                .unwrap_or(Ok(None))
        }
    }

    #[test]
    fn raw_traffic_records_returned_rx_before_decode_without_changing_bytes() {
        let transport = ScriptedTransport::default();
        let bytes = vec![0x99, 0x80, 0x7f];
        transport
            .reads
            .lock()
            .expect("reads")
            .push_back(Ok(Some(bytes.clone())));
        let journal = TrafficJournal::default();
        let observed = ObservedTransport::new(Box::new(transport.clone()), journal.clone());

        let read = observed.read_with_sequence(Duration::ZERO);

        assert_eq!(read.result.expect("read"), Some(bytes));
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        let event = journal
            .event(read.sequence.expect("RX sequence"))
            .expect("event");
        assert_eq!(&*event.retained_bytes, &[0x99, 0x80, 0x7f]);
        assert_eq!(event.reported_len, 3);
        assert_eq!(event.decode_status, Some(DecodeStatus::Pending));
        assert_eq!(event.classifier.query_category_at_8, None);
    }

    #[test]
    fn raw_traffic_records_write_outcomes_once_and_preserves_errors() {
        let transport = ScriptedTransport::default();
        let journal = TrafficJournal::default();
        let observed = ObservedTransport::new(Box::new(transport.clone()), journal.clone());
        observed.write(&[0x70, 1, 2, 3, 0x1d]).expect("write");
        *transport.write_error.lock().expect("write error") = Some("uncertain write".into());
        let error = observed.write(&[0x74]).expect_err("failed write");

        assert_eq!(error.to_string(), "uncertain write");
        assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
        let events = journal.window(None, None, usize::MAX);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].outcome, TrafficOutcome::WriteSucceeded);
        assert_eq!(events[0].classifier.command_opcode_at_4, Some(0x1d));
        assert_eq!(
            events[1].outcome,
            TrafficOutcome::WriteFailedDeliveryUncertain
        );
        assert_eq!(events[1].error.as_deref(), Some("uncertain write"));
    }

    #[test]
    fn raw_traffic_timeout_has_a_counter_but_no_ring_event() {
        let transport = ScriptedTransport::default();
        let journal = TrafficJournal::default();
        let observed = ObservedTransport::new(Box::new(transport), journal.clone());
        assert_eq!(observed.read(Duration::ZERO).expect("timeout"), None);
        assert_eq!(journal.stats().counters.read_timeouts, 1);
        assert_eq!(journal.stats().retained_events, 0);
    }

    #[test]
    fn raw_traffic_read_error_has_no_invented_bytes_and_clips_unicode_safely() {
        let transport = ScriptedTransport::default();
        transport
            .reads
            .lock()
            .expect("reads")
            .push_back(Err(anyhow!("é".repeat(TRAFFIC_ERROR_TEXT_BYTES_LIMIT))));
        let journal = TrafficJournal::default();
        let observed = ObservedTransport::new(Box::new(transport), journal.clone());
        assert!(observed.read(Duration::ZERO).is_err());

        let event = &journal.window(None, None, 1)[0];
        assert_eq!(event.outcome, TrafficOutcome::ReadFailed);
        assert_eq!(event.reported_len, 0);
        assert!(event.retained_bytes.is_empty());
        assert!(event.error_truncated);
        assert!(event.error.as_ref().expect("error").len() <= TRAFFIC_ERROR_TEXT_BYTES_LIMIT);
        assert!(std::str::from_utf8(event.error.as_ref().expect("error").as_bytes()).is_ok());
    }

    #[test]
    fn raw_traffic_classification_keeps_75_00_distinct_from_75_1f() {
        let mut query = vec![0; 13];
        query[0] = 0x75;
        query[8] = 0x0a;
        query[12] = 3;
        let mut meter = query.clone();
        meter[1] = 0x1f;
        assert_eq!(classify(&query).query_category_at_8, Some(0x0a));
        assert_eq!(classify(&query).query_index_at_12, Some(3));
        assert_eq!(classify(&meter).query_category_at_8, None);
        assert_eq!(classify(&meter).query_index_at_12, None);
    }

    #[test]
    fn raw_traffic_oversize_retains_only_prefix_and_returns_original_bytes() {
        let transport = ScriptedTransport::default();
        let bytes = vec![0x5a; TRAFFIC_EVENT_PAYLOAD_BYTES_LIMIT + 17];
        transport
            .reads
            .lock()
            .expect("reads")
            .push_back(Ok(Some(bytes.clone())));
        let journal = TrafficJournal::default();
        let observed = ObservedTransport::new(Box::new(transport), journal.clone());
        let read = observed.read_with_sequence(Duration::ZERO);
        assert_eq!(read.result.expect("read"), Some(bytes.clone()));

        let event = journal
            .event(read.sequence.expect("sequence"))
            .expect("event");
        assert_eq!(event.reported_len, bytes.len());
        assert_eq!(
            event.retained_bytes.len(),
            TRAFFIC_EVENT_PAYLOAD_BYTES_LIMIT
        );
        assert!(event.payload_truncated);
        assert_eq!(journal.stats().counters.payload_truncations, 1);
    }

    #[test]
    fn raw_traffic_enforces_count_and_payload_byte_limits_with_reasoned_evictions() {
        let count_journal = TrafficJournal::default();
        for _ in 0..=TRAFFIC_EVENT_COUNT_LIMIT {
            count_journal.record_read_returned(&[]);
        }
        let count_stats = count_journal.stats();
        assert_eq!(count_stats.retained_events, TRAFFIC_EVENT_COUNT_LIMIT);
        assert_eq!(count_stats.counters.evicted_by_count, 1);
        assert_eq!(count_stats.counters.evicted_by_bytes, 0);

        let byte_journal = TrafficJournal::default();
        let bytes = vec![0; TRAFFIC_EVENT_PAYLOAD_BYTES_LIMIT];
        for _ in 0..=(TRAFFIC_RETAINED_PAYLOAD_BYTES_LIMIT / bytes.len()) {
            byte_journal.record_read_returned(&bytes);
        }
        let byte_stats = byte_journal.stats();
        assert!(byte_stats.retained_payload_bytes <= TRAFFIC_RETAINED_PAYLOAD_BYTES_LIMIT);
        assert_eq!(byte_stats.counters.evicted_by_bytes, 1);
    }

    #[test]
    fn raw_traffic_counters_saturate_instead_of_wrapping() {
        let journal = TrafficJournal::default();
        {
            let mut inner = journal.lock();
            inner.counters.rx_returned = u64::MAX;
            inner.counters.read_timeouts = u64::MAX;
            inner.counters.payload_truncations = u64::MAX;
        }
        journal.record_timeout();
        journal.record_read_returned(&vec![0; TRAFFIC_EVENT_PAYLOAD_BYTES_LIMIT + 1]);
        let counters = journal.stats().counters;
        assert_eq!(counters.rx_returned, u64::MAX);
        assert_eq!(counters.read_timeouts, u64::MAX);
        assert_eq!(counters.payload_truncations, u64::MAX);
    }

    #[test]
    fn raw_traffic_window_is_bounded_and_selected_payload_is_shared() {
        let journal = TrafficJournal::default();
        for value in 0..=TRAFFIC_WINDOW_EVENT_LIMIT {
            journal.record_read_returned(&[value as u8]);
        }
        let window = journal.window(None, None, usize::MAX);
        assert_eq!(window.len(), TRAFFIC_WINDOW_EVENT_LIMIT);
        assert_eq!(
            window.last().expect("newest").sequence,
            TrafficSequence(257)
        );
        let selected = journal.event(window[0].sequence).expect("selected");
        assert!(Arc::ptr_eq(
            &window[0].retained_bytes,
            &selected.retained_bytes
        ));
        let ranged = journal.window(Some(window[0].sequence), Some(window[2].sequence), 10);
        assert_eq!(ranged.len(), 1);
        assert_eq!(ranged[0].sequence, window[1].sequence);
    }
}
