//! A timeline's read reports (`Timeline::report_visible`): the message rows
//! the viewport last reported visible, stamped with the version and
//! generation of the items the UI holds now, and whether the reader
//! scrolled there continuously or jumped (opened, restored, ↓, ⌛,
//! reopened), which the SDK's skipped blocks are built from.
//!
//! One report is on its way at a time; what changes meanwhile waits for its
//! answer. A refused report waits for new items, a failed one for other
//! rows. Whether a report may go at all (the window shown, no popup over the
//! rows) is the shell's to say; this keeps only what was sent.

use papo_sdk::{Generation, Uuid, VisibilityContinuity};

use super::{OpenTimeline, Phase};

/// What was reported, and what the next report says.
#[derive(Debug)]
pub(crate) struct Reports {
    /// The rows last reported, or on their way.
    sent: Vec<Uuid>,
    in_flight: bool,
    /// The reader jumped since the last report: the next is discontinuous.
    jumped: bool,
    /// A jump was asked for and its rows have not reported yet: the rows
    /// known now are from before it.
    awaiting_rows: bool,
    /// The items a report was refused for as stale: none goes again until
    /// they change.
    stale: Option<(u64, Generation)>,
}

impl Default for Reports {
    /// A new viewport: its first report is a jump to wherever it opened.
    fn default() -> Reports {
        Reports {
            sent: Vec::new(),
            in_flight: false,
            jumped: true,
            awaiting_rows: true,
            stale: None,
        }
    }
}

/// A read report to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Report {
    pub(crate) version: u64,
    pub(crate) generation: Generation,
    /// The visible message rows, top to bottom.
    pub(crate) ids: Vec<Uuid>,
    pub(crate) continuity: VisibilityContinuity,
}

/// How a report came back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Done,
    /// Refused: it was read from items the SDK has replaced since
    /// (`StaleSnapshot`). It goes again from the next items.
    Stale,
    Failed,
}

impl Reports {
    /// The reader jumps: the rows known now are from before the jump, and
    /// the first report after it is discontinuous.
    pub(crate) fn jumped(&mut self) {
        self.jumped = true;
        self.awaiting_rows = true;
    }

    /// The rows reported where they are.
    pub(crate) fn rows_reported(&mut self) {
        self.awaiting_rows = false;
    }

    /// The timeline opens another window: an answer still on its way is
    /// for the old one and is dropped, so nothing is on its way any more,
    /// and the new window's first report is a jump.
    pub(crate) fn reopened(&mut self) {
        self.in_flight = false;
        self.sent.clear();
        self.stale = None;
        self.jumped();
    }

    /// The report `visible` calls for, read from items at `version` and
    /// `generation`: none while one is on its way, before the rows of a
    /// jump report, or when they are the rows last reported.
    pub(crate) fn due(
        &mut self,
        visible: &[Uuid],
        version: u64,
        generation: Generation,
    ) -> Option<Report> {
        if self.in_flight
            || self.awaiting_rows
            || visible.is_empty()
            || self.sent == visible
            || self.stale == Some((version, generation))
        {
            return None;
        }
        self.stale = None;
        let continuity = if std::mem::take(&mut self.jumped) {
            VisibilityContinuity::Discontinuous
        } else {
            VisibilityContinuity::Continuous
        };
        self.in_flight = true;
        self.sent = visible.to_vec();
        Some(Report {
            version,
            generation,
            ids: visible.to_vec(),
            continuity,
        })
    }

    /// The report on its way came back as `report` says. A stale one goes
    /// again once the items change, a failed one once the rows do; either
    /// stays discontinuous if it was.
    pub(crate) fn answered(&mut self, report: &Report, outcome: Outcome) {
        self.in_flight = false;
        match outcome {
            Outcome::Done => return,
            Outcome::Stale => {
                self.sent.clear();
                self.stale = Some((report.version, report.generation));
            }
            Outcome::Failed => {}
        }
        if report.continuity == VisibilityContinuity::Discontinuous {
            self.jumped = true;
        }
    }
}

impl OpenTimeline {
    /// The read report the visible rows call for now, if any. None while
    /// no window is open, or while a place is still to be restored: the
    /// rows showing are not where the reader will be.
    pub(crate) fn report_due(&mut self) -> Option<Report> {
        if self.phase != Phase::Open || self.restore.is_some() {
            return None;
        }
        let generation = self.generation?;
        self.reports
            .due(self.tracker.visible(), self.version, generation)
    }
}

#[cfg(test)]
mod tests {
    use papo_sdk::{Generation, Uuid, VisibilityContinuity};

    use super::{Outcome, Reports};

    fn rows(ids: &[u128]) -> Vec<Uuid> {
        ids.iter().copied().map(Uuid::from_u128).collect()
    }

    fn reports_with_rows() -> Reports {
        let mut reports = Reports::default();
        reports.rows_reported();
        reports
    }

    #[test]
    fn the_first_report_is_a_jump_and_the_next_continue() {
        let mut reports = reports_with_rows();
        let generation = Generation::for_test(1);
        let first = reports.due(&rows(&[1, 2]), 4, generation).unwrap();
        assert_eq!(first.continuity, VisibilityContinuity::Discontinuous);
        assert_eq!((first.version, first.generation), (4, generation));
        reports.answered(&first, Outcome::Done);
        let next = reports.due(&rows(&[2, 3]), 5, generation).unwrap();
        assert_eq!(next.continuity, VisibilityContinuity::Continuous);
    }

    #[test]
    fn one_report_at_a_time_and_none_for_the_same_rows() {
        let mut reports = reports_with_rows();
        let generation = Generation::for_test(1);
        let first = reports.due(&rows(&[1]), 1, generation).unwrap();
        assert_eq!(
            reports.due(&rows(&[2]), 1, generation),
            None,
            "one on its way"
        );
        reports.answered(&first, Outcome::Done);
        assert_eq!(
            reports.due(&rows(&[1]), 2, generation),
            None,
            "already reported"
        );
        assert_eq!(reports.due(&[], 2, generation), None, "nothing visible");
        assert!(reports.due(&rows(&[2]), 2, generation).is_some());
    }

    #[test]
    fn a_jump_waits_for_its_own_rows() {
        let mut reports = reports_with_rows();
        let generation = Generation::for_test(1);
        let first = reports.due(&rows(&[1]), 1, generation).unwrap();
        reports.answered(&first, Outcome::Done);
        reports.jumped();
        assert_eq!(
            reports.due(&rows(&[2]), 1, generation),
            None,
            "the rows known are from before the jump"
        );
        reports.rows_reported();
        let after = reports.due(&rows(&[90]), 1, generation).unwrap();
        assert_eq!(after.continuity, VisibilityContinuity::Discontinuous);
    }

    #[test]
    fn a_stale_report_goes_again_from_the_next_items_as_it_was() {
        let mut reports = reports_with_rows();
        let generation = Generation::for_test(1);
        let first = reports.due(&rows(&[1]), 1, generation).unwrap();
        reports.answered(&first, Outcome::Stale);
        assert_eq!(
            reports.due(&rows(&[1]), 1, generation),
            None,
            "the same items"
        );
        let again = reports.due(&rows(&[1]), 2, generation).unwrap();
        assert_eq!(again.version, 2);
        assert_eq!(again.continuity, VisibilityContinuity::Discontinuous);
    }

    #[test]
    fn a_failed_report_waits_for_other_rows() {
        let mut reports = reports_with_rows();
        let generation = Generation::for_test(1);
        let first = reports.due(&rows(&[1]), 1, generation).unwrap();
        reports.answered(&first, Outcome::Failed);
        assert_eq!(
            reports.due(&rows(&[1]), 2, generation),
            None,
            "no hot retry"
        );
        assert!(reports.due(&rows(&[1, 2]), 2, generation).is_some());
    }

    #[test]
    fn reopening_drops_the_report_on_its_way() {
        let mut reports = reports_with_rows();
        let generation = Generation::for_test(1);
        reports.due(&rows(&[1]), 1, generation).unwrap();
        reports.reopened();
        assert_eq!(
            reports.due(&rows(&[1]), 1, generation),
            None,
            "waits for the new rows"
        );
        reports.rows_reported();
        let next = reports.due(&rows(&[1]), 1, generation).unwrap();
        assert_eq!(next.continuity, VisibilityContinuity::Discontinuous);
    }
}
