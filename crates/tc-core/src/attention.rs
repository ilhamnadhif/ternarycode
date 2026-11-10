//! The attention queue: panes waiting for the user, most urgent first, and the ⌘J / ⌘⇧J jumps
//! through them (FR-N1–N3).

use std::time::Instant;

use crate::PaneId;
use crate::status::Status;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub pane: PaneId,
    pub status: Status,
    pub since: Instant,
}

/// Orders entries: needs input, failed, done, command done; oldest first within each.
pub fn order(entries: &mut [Entry]) {
    entries.sort_by_key(|e| (e.status.priority().unwrap_or(u8::MAX), e.since));
}

/// Remembers where the first jump started, for ⌘⇧J.
#[derive(Clone, Debug, Default)]
pub struct Jumps {
    origin: Option<PaneId>,
}

impl Jumps {
    /// The pane ⌘J goes to: the most urgent entry that isn't the current pane. The first jump
    /// remembers where the user was.
    pub fn next(&mut self, current: Option<PaneId>, queue: &[Entry]) -> Option<PaneId> {
        let target = queue.iter().find(|e| Some(e.pane) != current)?.pane;
        if self.origin.is_none() {
            self.origin = current;
        }
        Some(target)
    }

    /// Where ⌘⇧J returns to (once).
    pub fn back(&mut self) -> Option<PaneId> {
        self.origin.take()
    }

    pub fn origin(&self) -> Option<PaneId> {
        self.origin
    }

    /// A pane was closed.
    pub fn forget(&mut self, pane: PaneId) {
        if self.origin == Some(pane) {
            self.origin = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn urgent_first_then_oldest_and_jumps_return_to_the_origin() {
        let t0 = Instant::now();
        let s = |n: u64| t0 + Duration::from_secs(n);
        let mut q = vec![
            Entry { pane: 1, status: Status::Done, since: s(1) },
            Entry { pane: 2, status: Status::CommandDone, since: s(0) },
            Entry { pane: 3, status: Status::NeedsInput, since: s(5) },
            Entry { pane: 4, status: Status::Failed(1), since: s(2) },
            Entry { pane: 5, status: Status::NeedsInput, since: s(3) },
        ];
        order(&mut q);
        assert_eq!(q.iter().map(|e| e.pane).collect::<Vec<_>>(), [5, 3, 4, 1, 2]);

        let mut j = Jumps::default();
        // From pane 9 to the most urgent.
        assert_eq!(j.next(Some(9), &q), Some(5));
        // Looking at 5 takes it out of the queue; the next jump goes on, origin unchanged.
        q.retain(|e| e.pane != 5);
        assert_eq!(j.next(Some(5), &q), Some(3));
        assert_eq!(j.origin(), Some(9));
        // ⌘⇧J goes back to where the user was, once.
        assert_eq!(j.back(), Some(9));
        assert_eq!(j.back(), None);
        // Nothing to jump to.
        assert_eq!(j.next(Some(1), &[]), None);
        // The current pane itself is skipped.
        let only = [Entry { pane: 7, status: Status::Done, since: s(0) }];
        assert_eq!(j.next(Some(7), &only), None);
    }
}
