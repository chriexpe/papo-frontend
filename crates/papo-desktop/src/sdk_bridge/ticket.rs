//! Request generations: a result for a request the UI has moved past is
//! dropped instead of applied.
//!
//! Owns only the counting. What a slot covers (the server step's probe, a
//! sign-in) is the owning view state's choice; delivering results is
//! `sdk_bridge`'s.

/// The generation of one request, as issued by its [`RequestSlot`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ticket(u64);

/// One kind of request, of which only the newest counts. Issuing a ticket
/// supersedes every ticket issued before it.
#[derive(Debug, Default)]
pub(crate) struct RequestSlot {
    current: u64,
}

impl RequestSlot {
    /// Starts a request; every earlier ticket from this slot is now stale.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the server step, the next slice, is the first to issue one"
        )
    )]
    pub(crate) fn issue(&mut self) -> Ticket {
        // No session issues 2^64 requests, so this cannot overflow.
        self.current += 1;
        Ticket(self.current)
    }

    /// Whether `ticket` is the newest this slot has issued.
    pub(crate) fn is_current(&self, ticket: Ticket) -> bool {
        ticket.0 == self.current
    }
}
