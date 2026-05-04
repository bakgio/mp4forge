//! Internal queue-backed work-item helpers for direct queue consumers.

use std::collections::BTreeSet;
#[cfg(any(feature = "decrypt", test))]
use std::collections::{HashMap, VecDeque};
#[cfg(any(feature = "decrypt", test))]
use std::fmt;

#[cfg(any(feature = "decrypt", test))]
use crate::FourCc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct QueueAuxiliaryInfoSpan {
    pub(crate) absolute_offset: u64,
    pub(crate) size: u64,
}

pub(crate) trait QueueWorkItem {
    fn queue_order_key(&self) -> u64;

    fn auxiliary_info_span(&self) -> Option<QueueAuxiliaryInfoSpan> {
        None
    }
}

#[cfg(any(feature = "decrypt", test))]
pub(crate) trait QueueRangeWorkItem: QueueWorkItem {
    fn queue_range_start(&self) -> u64;

    fn queue_range_size(&self) -> u64;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OrderedWorkQueue<T> {
    items: Vec<T>,
    auxiliary_info_spans: Vec<QueueAuxiliaryInfoSpan>,
}

impl<T> OrderedWorkQueue<T>
where
    T: QueueWorkItem,
{
    pub(crate) fn new(mut items: Vec<T>) -> Self {
        items.sort_by_key(QueueWorkItem::queue_order_key);

        let mut seen_auxiliary_info_spans = BTreeSet::new();
        let mut auxiliary_info_spans = Vec::new();
        for span in items.iter().filter_map(QueueWorkItem::auxiliary_info_span) {
            if seen_auxiliary_info_spans.insert(span) {
                auxiliary_info_spans.push(span);
            }
        }

        Self {
            items,
            auxiliary_info_spans,
        }
    }

    #[cfg(feature = "mux")]
    pub(crate) fn iter(&self) -> std::slice::Iter<'_, T> {
        self.items.iter()
    }

    #[cfg(feature = "decrypt")]
    pub(crate) fn items(&self) -> &[T] {
        &self.items
    }

    #[cfg(any(feature = "decrypt", test))]
    pub(crate) fn auxiliary_info_spans(&self) -> &[QueueAuxiliaryInfoSpan] {
        &self.auxiliary_info_spans
    }
}

#[cfg(any(feature = "decrypt", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RawOffsetQueueError {
    RangeOverflow { start: u64, size: u64 },
    RequestedClearedRange { start: u64, head: u64 },
    RequestedUnbufferedRange { end: u64, tail: u64 },
    TrimBeyondTail { target: u64, tail: u64 },
}

#[cfg(any(feature = "decrypt", test))]
impl fmt::Display for RawOffsetQueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RangeOverflow { start, size } => write!(
                f,
                "raw offset queue range at offset {start} with size {size} overflowed the supported range"
            ),
            Self::RequestedClearedRange { start, head } => write!(
                f,
                "raw offset queue request at offset {start} started before the current clear window head {head}"
            ),
            Self::RequestedUnbufferedRange { end, tail } => write!(
                f,
                "raw offset queue request ending at offset {end} exceeded the buffered tail {tail}"
            ),
            Self::TrimBeyondTail { target, tail } => write!(
                f,
                "raw offset queue clear-window target {target} exceeded the buffered tail {tail}"
            ),
        }
    }
}

#[cfg(any(feature = "decrypt", test))]
pub(crate) struct RawOffsetQueue {
    head: u64,
    bytes: VecDeque<u8>,
}

#[cfg(any(feature = "decrypt", test))]
impl RawOffsetQueue {
    pub(crate) fn new(head: u64) -> Self {
        Self {
            head,
            bytes: VecDeque::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn head(&self) -> u64 {
        self.head
    }

    pub(crate) fn tail(&self) -> u64 {
        self.head + u64::try_from(self.bytes.len()).unwrap()
    }

    #[cfg(test)]
    pub(crate) fn buffered_len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn push_bytes(&mut self, bytes: &[u8]) {
        self.bytes.extend(bytes.iter().copied());
    }

    pub(crate) fn trim_to(&mut self, target: u64) -> Result<(), RawOffsetQueueError> {
        if target < self.head {
            return Ok(());
        }
        let tail = self.tail();
        if target > tail {
            return Err(RawOffsetQueueError::TrimBeyondTail { target, tail });
        }
        let trim_len = usize::try_from(target - self.head)
            .map_err(|_| RawOffsetQueueError::TrimBeyondTail { target, tail })?;
        self.bytes.drain(..trim_len);
        self.head = target;
        Ok(())
    }

    pub(crate) fn with_range_bytes<T, F>(
        &mut self,
        start: u64,
        size: u64,
        read: F,
    ) -> Result<T, RawOffsetQueueError>
    where
        F: FnOnce(&[u8]) -> T,
    {
        let end = start
            .checked_add(size)
            .ok_or(RawOffsetQueueError::RangeOverflow { start, size })?;
        if start < self.head {
            return Err(RawOffsetQueueError::RequestedClearedRange {
                start,
                head: self.head,
            });
        }
        let tail = self.tail();
        if end > tail {
            return Err(RawOffsetQueueError::RequestedUnbufferedRange { end, tail });
        }

        let start_index = usize::try_from(start - self.head)
            .map_err(|_| RawOffsetQueueError::RangeOverflow { start, size })?;
        let len = usize::try_from(size)
            .map_err(|_| RawOffsetQueueError::RangeOverflow { start, size })?;
        let contiguous = self.bytes.make_contiguous();
        Ok(read(&contiguous[start_index..start_index + len]))
    }
}

#[cfg(any(feature = "decrypt", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RangeQueueParserStage<'a, T> {
    AuxiliaryInfo(&'a [QueueAuxiliaryInfoSpan]),
    CopyRange { start: u64, size: u64 },
    WorkItem(&'a T),
    Complete,
}

#[cfg(any(feature = "decrypt", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RangeQueueParserError {
    OverlappingWorkItemRange { next_start: u64, cursor: u64 },
    WorkItemRangeOverflow { start: u64, size: u64 },
}

#[cfg(any(feature = "decrypt", test))]
impl fmt::Display for RangeQueueParserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OverlappingWorkItemRange { next_start, cursor } => write!(
                f,
                "queue work item started at offset {next_start} before the parser cursor {cursor}"
            ),
            Self::WorkItemRangeOverflow { start, size } => write!(
                f,
                "queue work item at offset {start} with size {size} overflowed the supported range"
            ),
        }
    }
}

#[cfg(any(feature = "decrypt", test))]
pub(crate) struct RangeQueueParser<'a, T> {
    auxiliary_info_spans: &'a [QueueAuxiliaryInfoSpan],
    range_items: Vec<&'a T>,
    next_item_index: usize,
    pending_item: Option<&'a T>,
    cursor: u64,
    range_end: u64,
    emitted_auxiliary_info: bool,
    emitted_tail: bool,
}

#[cfg(any(feature = "decrypt", test))]
impl<'a, T> RangeQueueParser<'a, T>
where
    T: QueueRangeWorkItem,
{
    pub(crate) fn new(
        queue: Option<&'a OrderedWorkQueue<T>>,
        range_start: u64,
        range_end: u64,
    ) -> Self {
        let auxiliary_info_spans = queue
            .map(OrderedWorkQueue::auxiliary_info_spans)
            .unwrap_or(&[]);
        let mut range_items = queue
            .map(|queue| queue.items.iter().collect::<Vec<_>>())
            .unwrap_or_default();
        range_items.sort_by_key(|item| item.queue_range_start());
        Self {
            auxiliary_info_spans,
            range_items,
            next_item_index: 0,
            pending_item: None,
            cursor: range_start,
            range_end,
            emitted_auxiliary_info: false,
            emitted_tail: false,
        }
    }

    pub(crate) fn next_stage(
        &mut self,
    ) -> Result<RangeQueueParserStage<'a, T>, RangeQueueParserError> {
        if let Some(item) = self.pending_item.take() {
            self.cursor = checked_range_end(item.queue_range_start(), item.queue_range_size())?;
            return Ok(RangeQueueParserStage::WorkItem(item));
        }

        if !self.emitted_auxiliary_info {
            self.emitted_auxiliary_info = true;
            return Ok(RangeQueueParserStage::AuxiliaryInfo(
                self.auxiliary_info_spans,
            ));
        }

        if let Some(item) = self.range_items.get(self.next_item_index).copied() {
            self.next_item_index += 1;
            let item_start = item.queue_range_start();
            let item_size = item.queue_range_size();
            if item_start < self.cursor {
                return Err(RangeQueueParserError::OverlappingWorkItemRange {
                    next_start: item_start,
                    cursor: self.cursor,
                });
            }
            if item_start > self.cursor {
                self.pending_item = Some(item);
                return Ok(RangeQueueParserStage::CopyRange {
                    start: self.cursor,
                    size: item_start - self.cursor,
                });
            }
            self.cursor = checked_range_end(item_start, item_size)?;
            return Ok(RangeQueueParserStage::WorkItem(item));
        }

        if !self.emitted_tail && self.cursor < self.range_end {
            self.emitted_tail = true;
            return Ok(RangeQueueParserStage::CopyRange {
                start: self.cursor,
                size: self.range_end - self.cursor,
            });
        }
        Ok(RangeQueueParserStage::Complete)
    }
}

#[cfg(any(feature = "decrypt", test))]
fn checked_range_end(start: u64, size: u64) -> Result<u64, RangeQueueParserError> {
    start
        .checked_add(size)
        .ok_or(RangeQueueParserError::WorkItemRangeOverflow { start, size })
}

#[cfg(any(feature = "decrypt", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DecryptorReuseKey {
    scheme_type: FourCc,
    key_bytes: [u8; 16],
}

#[cfg(any(feature = "decrypt", test))]
impl DecryptorReuseKey {
    pub(crate) fn new(scheme_type: FourCc, key_bytes: [u8; 16]) -> Self {
        Self {
            scheme_type,
            key_bytes,
        }
    }
}

#[cfg(any(feature = "decrypt", test))]
pub(crate) struct DecryptorReuseCache<T> {
    entries: HashMap<DecryptorReuseKey, T>,
}

#[cfg(any(feature = "decrypt", test))]
impl<T> DecryptorReuseCache<T> {
    pub(crate) fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub(crate) fn touch_or_insert_with<F>(&mut self, key: DecryptorReuseKey, build: F) -> &mut T
    where
        F: FnOnce() -> T,
    {
        self.entries.entry(key).or_insert_with(build)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct TestWorkItem {
        queue_order_key: u64,
        queue_range_start: u64,
        auxiliary_info_span: Option<QueueAuxiliaryInfoSpan>,
    }

    impl QueueWorkItem for TestWorkItem {
        fn queue_order_key(&self) -> u64 {
            self.queue_order_key
        }

        fn auxiliary_info_span(&self) -> Option<QueueAuxiliaryInfoSpan> {
            self.auxiliary_info_span
        }
    }

    impl QueueRangeWorkItem for TestWorkItem {
        fn queue_range_start(&self) -> u64 {
            self.queue_range_start
        }

        fn queue_range_size(&self) -> u64 {
            4
        }
    }

    #[test]
    fn ordered_work_queue_sorts_items_and_preserves_first_auxiliary_stage_order() {
        let queue = OrderedWorkQueue::new(vec![
            TestWorkItem {
                queue_order_key: 44,
                queue_range_start: 44,
                auxiliary_info_span: Some(QueueAuxiliaryInfoSpan {
                    absolute_offset: 20,
                    size: 8,
                }),
            },
            TestWorkItem {
                queue_order_key: 12,
                queue_range_start: 12,
                auxiliary_info_span: Some(QueueAuxiliaryInfoSpan {
                    absolute_offset: 20,
                    size: 8,
                }),
            },
            TestWorkItem {
                queue_order_key: 31,
                queue_range_start: 31,
                auxiliary_info_span: Some(QueueAuxiliaryInfoSpan {
                    absolute_offset: 8,
                    size: 4,
                }),
            },
        ]);

        assert_eq!(
            queue
                .items
                .iter()
                .map(|item| item.queue_order_key)
                .collect::<Vec<_>>(),
            vec![12, 31, 44]
        );
        assert_eq!(
            queue.auxiliary_info_spans,
            vec![
                QueueAuxiliaryInfoSpan {
                    absolute_offset: 20,
                    size: 8,
                },
                QueueAuxiliaryInfoSpan {
                    absolute_offset: 8,
                    size: 4,
                },
            ]
        );
    }

    #[test]
    fn decryptor_reuse_cache_reuses_existing_entries_for_the_same_key() {
        let mut cache = DecryptorReuseCache::new();
        let key = DecryptorReuseKey::new(FourCc::from_bytes(*b"cenc"), [0x11; 16]);

        *cache.touch_or_insert_with(key, || 1_u32) += 1;
        *cache.touch_or_insert_with(key, || 9_u32) += 1;

        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.entries.get(&key), Some(&3_u32));
    }

    #[test]
    fn range_queue_parser_emits_copy_gaps_work_items_and_tail() {
        let queue = OrderedWorkQueue::new(vec![
            TestWorkItem {
                queue_order_key: 12,
                queue_range_start: 12,
                auxiliary_info_span: Some(QueueAuxiliaryInfoSpan {
                    absolute_offset: 4,
                    size: 2,
                }),
            },
            TestWorkItem {
                queue_order_key: 24,
                queue_range_start: 24,
                auxiliary_info_span: None,
            },
        ]);
        let mut parser = RangeQueueParser::new(Some(&queue), 8, 32);

        assert_eq!(
            parser.next_stage().unwrap(),
            RangeQueueParserStage::AuxiliaryInfo(&[QueueAuxiliaryInfoSpan {
                absolute_offset: 4,
                size: 2,
            }])
        );
        assert_eq!(
            parser.next_stage().unwrap(),
            RangeQueueParserStage::CopyRange { start: 8, size: 4 }
        );
        match parser.next_stage().unwrap() {
            RangeQueueParserStage::WorkItem(item) => assert_eq!(item.queue_order_key, 12),
            other => panic!("unexpected range queue stage: {other:?}"),
        }
        assert_eq!(
            parser.next_stage().unwrap(),
            RangeQueueParserStage::CopyRange { start: 16, size: 8 }
        );
        match parser.next_stage().unwrap() {
            RangeQueueParserStage::WorkItem(item) => assert_eq!(item.queue_order_key, 24),
            other => panic!("unexpected range queue stage: {other:?}"),
        }
        assert_eq!(
            parser.next_stage().unwrap(),
            RangeQueueParserStage::CopyRange { start: 28, size: 4 }
        );
        assert_eq!(
            parser.next_stage().unwrap(),
            RangeQueueParserStage::Complete
        );
    }

    #[test]
    fn range_queue_parser_rejects_overlapping_work_item_ranges() {
        let queue = OrderedWorkQueue::new(vec![
            TestWorkItem {
                queue_order_key: 12,
                queue_range_start: 12,
                auxiliary_info_span: None,
            },
            TestWorkItem {
                queue_order_key: 14,
                queue_range_start: 14,
                auxiliary_info_span: None,
            },
        ]);
        let mut parser = RangeQueueParser::new(Some(&queue), 8, 24);

        assert!(matches!(
            parser.next_stage().unwrap(),
            RangeQueueParserStage::AuxiliaryInfo(_)
        ));
        assert!(matches!(
            parser.next_stage().unwrap(),
            RangeQueueParserStage::CopyRange { start: 8, size: 4 }
        ));
        assert!(matches!(
            parser.next_stage().unwrap(),
            RangeQueueParserStage::WorkItem(_)
        ));
        assert_eq!(
            parser.next_stage().unwrap_err(),
            RangeQueueParserError::OverlappingWorkItemRange {
                next_start: 14,
                cursor: 16,
            }
        );
    }

    #[test]
    fn range_queue_parser_uses_range_order_even_when_queue_order_differs() {
        let queue = OrderedWorkQueue::new(vec![
            TestWorkItem {
                queue_order_key: 10,
                queue_range_start: 24,
                auxiliary_info_span: Some(QueueAuxiliaryInfoSpan {
                    absolute_offset: 6,
                    size: 2,
                }),
            },
            TestWorkItem {
                queue_order_key: 20,
                queue_range_start: 12,
                auxiliary_info_span: None,
            },
        ]);
        let mut parser = RangeQueueParser::new(Some(&queue), 8, 32);

        assert!(matches!(
            parser.next_stage().unwrap(),
            RangeQueueParserStage::AuxiliaryInfo(_)
        ));
        assert_eq!(
            parser.next_stage().unwrap(),
            RangeQueueParserStage::CopyRange { start: 8, size: 4 }
        );
        match parser.next_stage().unwrap() {
            RangeQueueParserStage::WorkItem(item) => {
                assert_eq!(item.queue_order_key, 20);
                assert_eq!(item.queue_range_start, 12);
            }
            other => panic!("unexpected range queue stage: {other:?}"),
        }
        assert_eq!(
            parser.next_stage().unwrap(),
            RangeQueueParserStage::CopyRange { start: 16, size: 8 }
        );
        match parser.next_stage().unwrap() {
            RangeQueueParserStage::WorkItem(item) => {
                assert_eq!(item.queue_order_key, 10);
                assert_eq!(item.queue_range_start, 24);
            }
            other => panic!("unexpected range queue stage: {other:?}"),
        }
    }

    #[test]
    fn raw_offset_queue_reads_and_trims_buffered_ranges() {
        let mut queue = RawOffsetQueue::new(100);
        queue.push_bytes(&[1, 2, 3, 4, 5, 6]);

        let copied = queue.with_range_bytes(102, 3, <[u8]>::to_vec).unwrap();
        assert_eq!(copied, vec![3, 4, 5]);
        assert_eq!(queue.head(), 100);
        assert_eq!(queue.tail(), 106);

        queue.trim_to(104).unwrap();
        assert_eq!(queue.head(), 104);
        assert_eq!(queue.tail(), 106);
        assert_eq!(queue.buffered_len(), 2);

        let remaining = queue.with_range_bytes(104, 2, <[u8]>::to_vec).unwrap();
        assert_eq!(remaining, vec![5, 6]);
    }

    #[test]
    fn raw_offset_queue_rejects_cleared_and_unbuffered_ranges() {
        let mut queue = RawOffsetQueue::new(40);
        queue.push_bytes(&[7, 8, 9, 10]);
        queue.trim_to(42).unwrap();

        assert_eq!(
            queue.with_range_bytes(41, 1, <[u8]>::to_vec).unwrap_err(),
            RawOffsetQueueError::RequestedClearedRange {
                start: 41,
                head: 42
            }
        );
        assert_eq!(
            queue.with_range_bytes(44, 2, <[u8]>::to_vec).unwrap_err(),
            RawOffsetQueueError::RequestedUnbufferedRange { end: 46, tail: 44 }
        );
        assert_eq!(
            queue.trim_to(45).unwrap_err(),
            RawOffsetQueueError::TrimBeyondTail {
                target: 45,
                tail: 44
            }
        );
    }
}
