//! What is fetched, in what order, what draws, and what is let go, from synthetic wants.

use super::*;
use papo_sdk::{AccountId, ServerAddress, Uuid};

fn key(n: u128) -> MediaKey {
    MediaKey::attachment(
        AccountId::new(
            ServerAddress::parse("https://lumen.example.invalid")
                .unwrap()
                .id(),
            Uuid::from_u128(8),
        ),
        Uuid::from_u128(n),
    )
}

fn want(n: u128, thumbnail_px: Option<(u32, u32)>) -> Want {
    Want {
        key: key(n),
        thumbnail_px,
        box_pt: 448.0,
        box_px: 896.0,
        gif: false,
    }
}

fn images() -> rune::ImageCache {
    rune::ImageCache::new(Arc::new(|| {}))
}

/// A decoded picture `side` pixels square.
fn picture(images: &rune::ImageCache, side: u32, shade: u8) -> Image {
    let bytes = vec![shade; (side * side * 4) as usize];
    let image = images.rgba(side, side, bytes);
    let _ = image.wait();
    image
}

fn fetches(jobs: &[Job]) -> Vec<(u128, Fetch)> {
    jobs.iter()
        .map(|job| (job.key.id.as_u128(), job.fetch))
        .collect()
}

#[test]
fn a_thumbnail_as_large_as_the_box_is_enough() {
    let mut store = MediaStore::default();
    let jobs = store.want([want(1, Some((1024, 664)))]);
    assert_eq!(fetches(&jobs), [(1, Fetch::Thumbnail)]);
    let jobs = store.landed(jobs[0], Some(picture(&images(), 4, 1)), 0);
    assert!(jobs.is_empty(), "no original");
    assert!(
        store
            .view()
            .get(key(1))
            .is_some_and(|drawn| drawn.picture(false).is_some())
    );
}

#[test]
fn a_small_thumbnail_shows_while_the_original_loads() {
    let mut store = MediaStore::default();
    let jobs = store.want([want(1, Some((512, 384)))]);
    let jobs = store.landed(jobs[0], Some(picture(&images(), 4, 1)), 0);
    assert_eq!(fetches(&jobs), [(1, Fetch::Original)]);
    let before = store.view().get(key(1)).map(|drawn| drawn.version);
    let jobs = store.landed(jobs[0], Some(picture(&images(), 8, 2)), 0);
    assert_eq!(jobs, Vec::<Job>::new());
    let view = store.view();
    let drawn = view.get(key(1)).expect("drawn");
    assert_ne!(Some(drawn.version), before);
    assert_eq!(drawn.picture(false).and_then(Image::size), Some([8, 8]));
}

#[test]
fn without_a_thumbnail_the_original_is_fetched() {
    let mut store = MediaStore::default();
    assert_eq!(
        fetches(&store.want([want(1, None)])),
        [(1, Fetch::Original)]
    );
}

#[test]
fn two_fetches_run_at_once_and_rows_that_left_are_dropped() {
    let mut store = MediaStore::default();
    let jobs = store.want([want(1, None), want(2, None), want(3, None)]);
    assert_eq!(fetches(&jobs), [(1, Fetch::Original), (2, Fetch::Original)]);
    // Row 3 scrolled away before its fetch started; row 4 came.
    assert!(store.want([want(4, None)]).is_empty(), "both slots busy");
    let next = store.landed(jobs[0], Some(picture(&images(), 2, 1)), 0);
    assert_eq!(fetches(&next), [(4, Fetch::Original)]);
    // Row 3 returns: asked again.
    let next = store.landed(jobs[1], Some(picture(&images(), 2, 1)), 0);
    assert_eq!(next, Vec::<Job>::new());
    assert_eq!(
        fetches(&store.want([want(3, None), want(4, None)])),
        [(3, Fetch::Original)]
    );
}

#[test]
fn a_failed_original_fails_and_retry_asks_again() {
    let mut store = MediaStore::default();
    let jobs = store.want([want(1, None)]);
    assert_eq!(store.landed(jobs[0], None, 0), Vec::<Job>::new());
    assert!(store.view().get(key(1)).is_some_and(|drawn| drawn.failed));
    let jobs = store.retry(key(1));
    assert_eq!(fetches(&jobs), [(1, Fetch::Original)]);
    assert!(store.view().get(key(1)).is_some_and(|drawn| !drawn.failed));
}

#[test]
fn a_failed_thumbnail_falls_back_on_the_original() {
    let mut store = MediaStore::default();
    let jobs = store.want([want(1, Some((1024, 664)))]);
    let jobs = store.landed(jobs[0], None, 0);
    assert_eq!(fetches(&jobs), [(1, Fetch::Original)]);
    assert!(store.view().get(key(1)).is_some_and(|drawn| !drawn.failed));
}

#[test]
fn a_gif_animates_while_it_shows_and_stays_warm_after() {
    let mut store = MediaStore::default();
    let mut gif = want(1, Some((512, 384)));
    gif.gif = true;
    let jobs = store.want([gif]);
    store.landed(jobs[0], Some(picture(&images(), 4, 1)), 0);
    let jobs = store.show_gifs(vec![key(1)]);
    let animation = jobs
        .iter()
        .copied()
        .find(|job| job.fetch == Fetch::Animation)
        .expect("its animation is fetched");
    assert!(!store.view().animates(key(1)), "not before it decoded");
    store.landed(animation, Some(picture(&images(), 4, 2)), 1000);
    assert!(store.view().animates(key(1)));
    store.show_gifs(Vec::new());
    assert!(!store.view().animates(key(1)), "stopped off screen");
    assert!(
        store
            .view()
            .get(key(1))
            .is_some_and(|drawn| drawn.animated.is_some()),
        "kept warm"
    );
}

#[test]
fn animations_past_the_budget_show_still() {
    let mut store = MediaStore::with_budgets(Budgets {
        still_warm_bytes: u64::MAX,
        animated_warm_bytes: 0,
        animating_bytes: 1500,
    });
    let gifs: Vec<Want> = (1..=2)
        .map(|n| Want {
            gif: true,
            // A thumbnail as large as the box: no original competes for the fetches.
            ..want(n, Some((1024, 1024)))
        })
        .collect();
    for job in store.want(gifs.clone()) {
        store.landed(job, Some(picture(&images(), 2, 1)), 0);
    }
    let jobs = store.show_gifs(vec![key(1), key(2)]);
    for job in jobs {
        store.landed(job, Some(picture(&images(), 2, 2)), 1000);
    }
    assert!(store.view().animates(key(1)));
    assert!(!store.view().animates(key(2)), "past the budget");
    // Nothing animates once off screen, and none is kept warm at a zero budget.
    store.show_gifs(Vec::new());
    let view = store.view();
    assert!((1..=2).all(|n| {
        view.get(key(n))
            .is_some_and(|drawn| drawn.animated.is_none())
    }));
}

#[test]
fn stills_no_view_draws_are_let_go_past_the_budget_oldest_first() {
    // Each 8 × 8 picture counts 8 × 8 × 4 × 4 / 3 = 341 bytes.
    let mut store = MediaStore::with_budgets(Budgets {
        still_warm_bytes: 700,
        animated_warm_bytes: 0,
        animating_bytes: 0,
    });
    for n in 1..=3 {
        for job in store.want([want(n, None)]) {
            store.landed(job, Some(picture(&images(), 8, 1)), 0);
        }
    }
    store.want([want(4, None)]);
    let view = store.view();
    assert!(view.get(key(1)).is_none(), "the oldest is let go");
    assert!(view.get(key(2)).is_some() && view.get(key(3)).is_some());
}

#[test]
fn a_much_larger_box_asks_for_the_picture_again() {
    let mut store = MediaStore::default();
    let jobs = store.want([want(1, None)]);
    store.landed(jobs[0], Some(picture(&images(), 4, 1)), 0);
    let mut larger = want(1, None);
    larger.box_px = 1_400.0;
    assert_eq!(fetches(&store.want([larger])), [(1, Fetch::Original)]);
    // The old picture draws until the new one lands.
    assert!(
        store
            .view()
            .get(key(1))
            .is_some_and(|drawn| drawn.picture(false).is_some())
    );
    larger.box_px = 1_450.0;
    assert!(store.want([larger]).is_empty(), "not for a few pixels more");
}

#[test]
fn a_gif_shown_before_its_row_is_wanted_still_animates() {
    let mut store = MediaStore::default();
    assert!(
        store.show_gifs(vec![key(1)]).is_empty(),
        "nothing wants it yet"
    );
    let gif = Want {
        gif: true,
        ..want(1, Some((1024, 1024)))
    };
    let jobs = store.want([gif]);
    assert!(
        fetches(&jobs).contains(&(1, Fetch::Animation)),
        "{:?}",
        fetches(&jobs)
    );
}

#[test]
fn a_fetch_running_is_not_started_twice_and_unstarted_ones_free_their_slots() {
    let mut store = MediaStore::default();
    let jobs = store.want([want(1, None), want(2, None)]);
    // Both leave view while fetching, come back: nothing starts again.
    store.want([want(3, None)]);
    assert!(
        store.want([want(1, None), want(2, None)]).is_empty(),
        "already running"
    );
    // The SDK was not there: the slots are free again.
    store.dropped(&jobs);
    assert_eq!(
        fetches(&store.want([want(1, None), want(2, None)])),
        [(1, Fetch::Original), (2, Fetch::Original)]
    );
}

#[test]
fn a_failed_animation_is_not_asked_again() {
    let mut store = MediaStore::default();
    let gif = Want {
        gif: true,
        ..want(1, Some((1024, 1024)))
    };
    for job in store.want([gif]) {
        store.landed(job, Some(picture(&images(), 2, 1)), 0);
    }
    let animation = store
        .show_gifs(vec![key(1)])
        .into_iter()
        .find(|job| job.fetch == Fetch::Animation)
        .expect("asked once");
    store.landed(animation, None, 0);
    store.show_gifs(Vec::new());
    assert!(store.show_gifs(vec![key(1)]).is_empty(), "not asked again");
}

#[test]
fn a_refetch_that_fails_keeps_the_picture_it_had() {
    let mut store = MediaStore::default();
    let jobs = store.want([want(1, None)]);
    store.landed(jobs[0], Some(picture(&images(), 4, 1)), 0);
    let mut larger = want(1, None);
    larger.box_px = 1_400.0;
    let refetch = store.want([larger]);
    store.landed(refetch[0], None, 0);
    let drawn = store.view().get(key(1)).cloned().expect("drawn");
    assert!(
        !drawn.failed && drawn.picture(false).is_some(),
        "still the old picture"
    );
    assert!(
        store.want([larger]).is_empty(),
        "not asked again at the same size"
    );
}
