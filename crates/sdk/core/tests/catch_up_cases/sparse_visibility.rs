//! Behaviour spec verifying sparse discontinuous reports do not consume omitted interior rows.

use super::{
    Fixture, Result, channel, config, interval, load_history, msg_id, msg_point, report_range,
    signed_in,
};
use papo_sdk::VisibilityContinuity;

#[test]
fn sparse_discontinuous_report_on_loaded_history_keeps_unreported_interior_in_catch_up() -> Result {
    let fixture = Fixture::new(config(10))?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        let snap = load_history(&session, channel()).await?;
        report_range(&session, channel(), 0..=0, VisibilityContinuity::Continuous).await?;
        report_range(
            &session,
            channel(),
            8..=9,
            VisibilityContinuity::Discontinuous,
        )
        .await?;
        // Report only [m2, m8] discontinuously on a fully loaded chunk: [m3..=m7] stay unread.
        let sparse = [msg_id(&snap, 1), msg_id(&snap, 7)];
        session
            .report_visible(
                channel(),
                snap.generation,
                snap.version,
                &sparse,
                VisibilityContinuity::Discontinuous,
            )
            .await?;
        let gap = interval(&session, channel()).expect("interior m3..=m7 must remain in catch-up");
        assert_eq!(gap.start_after, Some(msg_point(&snap, 1)));
        assert_eq!(gap.end_before, msg_point(&snap, 7));
        assert_eq!(gap.covered_spans, vec![]);
        assert_eq!(
            session.catch_up_view(channel(), &snap)?.start_target,
            Some(msg_id(&snap, 2))
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
