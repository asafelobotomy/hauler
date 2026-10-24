//! Segment bookkeeping and dynamic splitting.
//!
//! A file is a set of half-open byte ranges. Each segment tracks two cursors:
//! `received` (bytes taken off the socket, possibly still buffered) and
//! `written` (bytes on disk). When a connection is free and no segment is
//! waiting, the planner halves the segment with the most bytes left to
//! receive and hands the upper half to the free connection. The current owner
//! notices its shrunken `end` and stops early. Splitting happens at or above
//! `received`, so buffered data never crosses a split point.

use crate::state::SegmentState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub start: u64,
    pub end: u64,
    pub received: u64,
    pub written: u64,
    pub owner: Option<usize>,
}

impl Segment {
    fn new(start: u64, end: u64) -> Self {
        Self { start, end, received: start, written: start, owner: None }
    }

    pub fn remaining(&self) -> u64 {
        self.end.saturating_sub(self.received)
    }

    pub fn is_complete(&self) -> bool {
        self.written >= self.end
    }
}

/// A segment as a progress display sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentView {
    pub start: u64,
    pub end: u64,
    /// Received so far, from `start`.
    pub done: u64,
    /// A connection is working on it.
    pub active: bool,
}

#[derive(Debug)]
pub struct Plan {
    segments: Vec<Segment>,
    min_split: u64,
    splits: u64,
}

impl Plan {
    /// Cut `[0, size)` into up to `parts` equal segments no smaller than
    /// `min_split`.
    pub fn new(size: u64, parts: usize, min_split: u64) -> Self {
        let min_split = min_split.max(1);
        let parts = (parts.max(1) as u64).min(size.div_ceil(min_split)).max(1);
        let step = size.div_ceil(parts);
        let segments = (0..parts)
            .map(|i| Segment::new(i * step, ((i + 1) * step).min(size)))
            .filter(|s| s.start < s.end)
            .collect();
        Self { segments, min_split, splits: 0 }
    }

    /// Rebuild from saved state. Unfinished bytes past `written` are fetched
    /// again, since anything not yet on disk was lost with the process.
    pub fn from_state(saved: &[SegmentState], min_split: u64) -> Self {
        let segments = saved
            .iter()
            .map(|s| Segment { start: s.start, end: s.end, received: s.written, written: s.written, owner: None })
            .collect();
        Self { segments, min_split: min_split.max(1), splits: 0 }
    }

    pub fn segment(&self, idx: usize) -> &Segment {
        &self.segments[idx]
    }

    pub fn segment_mut(&mut self, idx: usize) -> &mut Segment {
        &mut self.segments[idx]
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn splits(&self) -> u64 {
        self.splits
    }

    /// Give `worker` a segment: an unowned unfinished one if any, otherwise
    /// the upper half of the largest segment still being received.
    pub fn claim(&mut self, worker: usize) -> Option<usize> {
        if let Some(idx) = self.segments.iter().position(|s| s.owner.is_none() && !s.is_complete()) {
            self.segments[idx].owner = Some(worker);
            return Some(idx);
        }

        let (idx, largest) =
            self.segments.iter().enumerate().filter(|(_, s)| s.owner.is_some()).max_by_key(|(_, s)| s.remaining())?;
        let remaining = largest.remaining();
        if remaining < 2 * self.min_split {
            return None;
        }
        let mid = largest.received + remaining / 2;
        let end = largest.end;
        self.segments[idx].end = mid;
        let mut new = Segment::new(mid, end);
        new.owner = Some(worker);
        self.segments.push(new);
        self.splits += 1;
        Some(self.segments.len() - 1)
    }

    /// Hand a segment back. Unwritten bytes are dropped so whoever claims it
    /// next starts from what is on disk.
    pub fn release(&mut self, idx: usize) {
        let s = &mut self.segments[idx];
        s.owner = None;
        s.received = s.written;
    }

    pub fn is_complete(&self) -> bool {
        self.segments.iter().all(Segment::is_complete)
    }

    pub fn written(&self) -> u64 {
        self.segments.iter().map(|s| s.written - s.start).sum()
    }

    /// Segments in file order, for drawing a progress map.
    pub fn view(&self) -> Vec<SegmentView> {
        let mut v: Vec<_> = self
            .segments
            .iter()
            .map(|s| SegmentView { start: s.start, end: s.end, done: s.received, active: s.owner.is_some() })
            .collect();
        v.sort_by_key(|s| s.start);
        v
    }

    pub fn snapshot(&self) -> Vec<SegmentState> {
        self.segments.iter().map(|s| SegmentState { start: s.start, end: s.end, written: s.written }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn splits_evenly_and_respects_min_split() {
        let plan = Plan::new(10 * MIB, 4, MIB);
        let bounds: Vec<_> = plan.segments().iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(bounds.len(), 4);
        assert_eq!(bounds[0].0, 0);
        assert_eq!(bounds[3].1, 10 * MIB);
        assert!(bounds.windows(2).all(|w| w[0].1 == w[1].0));

        assert_eq!(Plan::new(3 * MIB, 16, MIB).segments().len(), 3);
        assert_eq!(Plan::new(10, 8, MIB).segments().len(), 1);
    }

    #[test]
    fn free_worker_halves_the_largest_remaining_segment() {
        let mut plan = Plan::new(8 * MIB, 2, MIB);
        assert_eq!(plan.claim(0), Some(0));
        assert_eq!(plan.claim(1), Some(1));
        plan.segment_mut(1).received = 4 * MIB + MIB; // segment 1 has 3 MiB left
        // Segment 0 has 4 MiB left, so it is split at 2 MiB.
        assert_eq!(plan.claim(2), Some(2));
        assert_eq!(plan.segment(0).end, 2 * MIB);
        assert_eq!((plan.segment(2).start, plan.segment(2).end), (2 * MIB, 4 * MIB));
        assert_eq!(plan.splits(), 1);
    }

    #[test]
    fn split_never_cuts_below_received() {
        let mut plan = Plan::new(8 * MIB, 1, MIB);
        plan.claim(0);
        plan.segment_mut(0).received = 6 * MIB;
        let idx = plan.claim(1).unwrap();
        assert_eq!(plan.segment(idx).start, 7 * MIB);
        assert!(plan.segment(0).end >= plan.segment(0).received);
    }

    #[test]
    fn no_split_below_twice_min_split() {
        let mut plan = Plan::new(3 * MIB, 1, 2 * MIB);
        plan.claim(0);
        assert_eq!(plan.claim(1), None);
    }

    #[test]
    fn release_and_resume_restart_from_written() {
        let mut plan = Plan::new(4 * MIB, 1, MIB);
        plan.claim(0);
        plan.segment_mut(0).received = 3 * MIB;
        plan.segment_mut(0).written = 2 * MIB;
        plan.release(0);
        assert_eq!(plan.segment(0).received, 2 * MIB);

        let restored = Plan::from_state(&plan.snapshot(), MIB);
        assert_eq!(restored.written(), 2 * MIB);
        assert!(!restored.is_complete());
    }
}
