//! The GIF tab's state: where it opens, the pause before a search, GIPHY's pages and late
//! answers, the no-key and unreachable states, and the keyboard over the tiles.

use papo_sdk::Gif;

use super::{Ask, GifPicker, Giphy, Page, Paging, SEARCH_AFTER_S, Shelf};

fn gif(id: &str) -> Gif {
    Gif {
        id: id.to_owned(),
        ..Gif::default()
    }
}

fn page(ids: &[&str], next_offset: Option<u32>) -> Page {
    Page {
        gifs: ids.iter().map(|id| gif(id)).collect(),
        next_offset,
    }
}

#[test]
fn it_opens_on_favorites_when_there_are_any_else_on_trending() {
    let with = GifPicker::new(true, vec![gif("f")], Vec::new());
    assert_eq!(with.shelf, Shelf::Favorites);
    let without = GifPicker::new(true, Vec::new(), Vec::new());
    assert_eq!(without.shelf, Shelf::Trending);
    let no_key = GifPicker::new(false, Vec::new(), vec![gif("r")]);
    assert_eq!(no_key.shelf, Shelf::Favorites, "no Trending without a key");
    assert_eq!(no_key.giphy, Giphy::NoKey);
}

#[test]
fn recent_keeps_the_last_eight() {
    let recent = (0..12).map(|n| gif(&n.to_string())).collect();
    let mut picker = GifPicker::new(true, Vec::new(), recent);
    assert_eq!(picker.choose(Shelf::Recent), None, "nothing to ask GIPHY");
    assert_eq!(picker.gifs().len(), 8);
}

#[test]
fn a_search_waits_for_a_pause_in_typing_and_a_late_answer_is_dropped() {
    let mut picker = GifPicker::new(true, vec![gif("f")], Vec::new());
    picker.search.replace_text("ca");
    assert_eq!(picker.typed(1.0), None);
    picker.search.replace_text("cat");
    assert_eq!(picker.typed(1.2), None);
    assert_eq!(picker.due(1.3), None, "still typing");
    let ask = picker.due(1.2 + SEARCH_AFTER_S).expect("the search");
    assert_eq!(ask.query.as_deref(), Some("cat"));
    assert!(picker.first_page_loading());
    // A newer search goes out before the first answers: the first answer is dropped.
    picker.search.replace_text("cats");
    picker.typed(2.0);
    let newer = picker.due(2.0 + SEARCH_AFTER_S).unwrap();
    assert!(!picker.landed(ask.tag, Some(page(&["old"], None))));
    assert!(picker.landed(newer.tag, Some(page(&["a", "b"], Some(50)))));
    assert_eq!(picker.gifs().len(), 2);
}

#[test]
fn the_next_page_is_asked_for_near_the_end_once_at_a_time() {
    let mut picker = GifPicker::new(true, Vec::new(), Vec::new());
    let first = picker.choose(Shelf::Trending).unwrap();
    assert_eq!(picker.near_end(), None, "the first page is still coming");
    picker.landed(first.tag, Some(page(&["a"], Some(50))));
    let next = picker.near_end().unwrap();
    assert_eq!(next.offset, 50);
    assert!(picker.more_loading());
    assert_eq!(picker.near_end(), None, "one at a time");
    picker.landed(next.tag, Some(page(&["b"], None)));
    assert_eq!(picker.gifs().len(), 2);
    assert_eq!(picker.near_end(), None, "GIPHY has no more");
}

#[test]
fn unreachable_giphy_says_so_try_again_asks_again_and_another_shelf_asks_anew() {
    let mut picker = GifPicker::new(true, vec![gif("f")], Vec::new());
    let ask = picker.choose(Shelf::Trending).unwrap();
    assert!(picker.landed(ask.tag, None));
    assert_eq!(picker.giphy, Giphy::Unreachable);
    let again = picker.try_again().expect("asked again");
    assert_eq!((again.offset, picker.paging), (0, Paging::Loading));
    assert!(picker.landed(again.tag, None));
    assert_eq!(picker.choose(Shelf::Favorites), None);
    assert_eq!(picker.gifs().len(), 1, "favorites still work");
    let anew = picker.choose(Shelf::Search("cat".to_owned()));
    assert!(anew.is_some(), "a new search asks without Try again");
    assert_eq!(picker.giphy, Giphy::Ready);
}

#[test]
fn a_search_chip_fills_the_field_and_emptying_it_goes_back_to_trending() {
    let mut picker = GifPicker::new(true, Vec::new(), Vec::new());
    let ask = picker.choose(Shelf::Search("dogs".to_owned())).unwrap();
    assert_eq!(picker.search.text, "dogs");
    assert_eq!(ask.query.as_deref(), Some("dogs"));
    picker.search.replace_text("");
    let back: Option<Ask> = picker.typed(5.0);
    assert_eq!(picker.shelf, Shelf::Trending);
    assert!(back.is_some_and(|ask| ask.query.is_none()));
}

#[test]
fn without_a_key_typing_asks_nothing() {
    let mut picker = GifPicker::new(false, Vec::new(), Vec::new());
    picker.search.replace_text("cat");
    assert_eq!(picker.typed(1.0), None);
    assert_eq!(picker.due(9.0), None);
}

#[test]
fn the_keyboard_walks_the_tiles_and_goes_back_up_to_the_search() {
    let mut picker = GifPicker::new(true, vec![gif("a"), gif("b")], Vec::new());
    assert_eq!(picker.step(1), Some(0));
    assert_eq!(picker.step(1), Some(1));
    assert_eq!(picker.step(1), Some(1), "the last stays");
    assert_eq!(picker.chosen().map(|gif| gif.id.as_str()), Some("b"));
    assert_eq!(picker.step(-2), None, "back to the search");
}

#[test]
fn up_and_down_follow_the_tiles_column() {
    // Squares: tile 3 goes under tile 0 in the first column, tile 4 under tile 1.
    let gifs = (0..5)
        .map(|n| Gif {
            id: n.to_string(),
            preview_width: 100,
            preview_height: 100,
            ..Gif::default()
        })
        .collect();
    let mut picker = GifPicker::new(true, gifs, Vec::new());
    assert_eq!(picker.step_vertical(true), Some(0), "into the tiles");
    assert_eq!(picker.step_vertical(true), Some(3));
    assert_eq!(
        picker.step_vertical(true),
        Some(3),
        "the column's last stays"
    );
    assert_eq!(picker.step(1), Some(4));
    assert_eq!(picker.step_vertical(false), Some(1));
    assert_eq!(
        picker.step_vertical(false),
        None,
        "up from the top: the search"
    );
    picker.step(1);
    assert_eq!(picker.step_to_end(true), Some(4));
    assert_eq!(picker.step_to_end(false), Some(0));
}
