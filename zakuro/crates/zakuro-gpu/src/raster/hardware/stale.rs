//! the spans of a surface's bytes gone stale, where memory is newer than
//! what the surface drew.

/// the most spans apart a surface keeps stale, past which what it drew comes
/// down instead. Kid Icarus Uprising's scene depth has two, under its two
/// bloom buffers.
const MOST_STALE_SPANS: usize = 8;

/// spans of a surface's bytes, from its start, apart and in order, joined
/// where they meet. nothing is allocated while there are none.
#[derive(Clone, Default, PartialEq, Eq)]
pub(super) struct Stale(Vec<(u32, u32)>);

impl std::fmt::Debug for Stale {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_list().entries(&self.0).finish()
    }
}

impl Stale {
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn clear(&mut self) {
        self.0.clear();
    }

    pub(super) fn spans(&self) -> &[(u32, u32)] {
        &self.0
    }

    /// the spans once another joins them, none when that leaves more apart
    /// than MOST_STALE_SPANS.
    pub(super) fn with(&self, (start, end): (u32, u32)) -> Option<Stale> {
        if start >= end {
            return Some(self.clone());
        }
        let (mut start, mut end) = (start, end);
        let mut spans = Vec::with_capacity(self.0.len() + 1);
        for &(from, to) in &self.0 {
            if to < start || end < from {
                spans.push((from, to));
            } else {
                (start, end) = (start.min(from), end.max(to));
            }
        }
        let at = spans.partition_point(|&(from, _)| from < start);
        spans.insert(at, (start, end));
        (spans.len() <= MOST_STALE_SPANS).then_some(Stale(spans))
    }

    /// whether any of them overlaps the bytes from start to end.
    pub(super) fn overlaps(&self, start: u32, end: u32) -> bool {
        self.0.iter().any(|&(from, to)| from < end && start < to)
    }

    /// the bytes from start to end outside them, a piece at a time, until
    /// each says true, and whether one did.
    pub(super) fn any_outside(&self, start: u32, end: u32, mut each: impl FnMut(u32, u32) -> bool) -> bool {
        let mut at = start;
        for &(from, to) in &self.0 {
            if to <= at {
                continue;
            }
            if from >= end {
                break;
            }
            if from > at && each(at, from) {
                return true;
            }
            at = at.max(to);
            if at >= end {
                return false;
            }
        }
        at < end && each(at, end)
    }

    /// those pieces.
    pub(super) fn outside(&self, start: u32, end: u32) -> Vec<(u32, u32)> {
        let mut pieces = Vec::new();
        self.any_outside(start, end, |from, to| {
            pieces.push((from, to));
            false
        });
        pieces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// stale spans stay in order and apart, join where they meet or
    /// overlap, leave the bytes between them to be told apart, and no more
    /// than MOST_STALE_SPANS of them are kept.
    #[test]
    fn stale_spans_join_where_they_meet_and_no_more_are_kept() {
        let stale = Stale::default().with((300, 400)).unwrap().with((0, 100)).unwrap();
        assert_eq!(stale.spans(), [(0, 100), (300, 400)]);
        // meeting one, then overlapping both
        let met = stale.with((100, 150)).unwrap();
        assert_eq!(met.spans(), [(0, 150), (300, 400)]);
        assert_eq!(met.with((120, 320)).unwrap().spans(), [(0, 400)]);
        // the bytes outside them, and whether any overlap
        assert_eq!(stale.outside(50, 500), [(100, 300), (400, 500)]);
        assert!(stale.outside(0, 100).is_empty());
        assert!(stale.overlaps(99, 101) && !stale.overlaps(100, 300));
        // nothing added for no bytes
        assert_eq!(stale.with((200, 200)).unwrap(), stale);
        let mut many = Stale::default();
        for k in 0..MOST_STALE_SPANS as u32 {
            many = many.with((k * 10, k * 10 + 5)).unwrap();
        }
        assert!(many.with((1000, 1005)).is_none(), "one more apart than kept");
        assert!(many.with((5, 10)).is_some(), "joining two keeps them as many");
    }
}
