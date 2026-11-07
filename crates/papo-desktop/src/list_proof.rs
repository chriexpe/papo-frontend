//! Headless GPUI Fast integration proof for SDK vector-diff batches.
//! This test-only module owns list rendering and scroll anchoring, not SDK state.

use std::sync::Arc;

use gpui::{
    AppContext as _, Context, IntoElement, ListAlignment, ListOffset, ListState, Render, Window,
    div, list, px,
};
use papo_sdk_ui::integration_proof::{ProofRow, VectorDiff, batches};

struct ProofList {
    state: ListState,
    items: Vec<Arc<ProofRow>>,
}

impl ProofList {
    fn new() -> Self {
        Self {
            state: ListState::new(0, ListAlignment::Top, px(0.)),
            items: Vec::new(),
        }
    }

    /// Apply only the touched range; a reset keeps the same message ID anchored.
    fn apply(&mut self, changes: Vec<VectorDiff>) {
        for change in changes {
            match change {
                VectorDiff::Insert { index, value } => {
                    self.items.insert(index, Arc::new(value));
                    self.state.splice(index..index, 1);
                }
                VectorDiff::Set { index, value } => {
                    self.items[index] = Arc::new(value);
                    self.state.splice(index..index + 1, 1);
                }
                VectorDiff::PushBack { value } => {
                    let index = self.items.len();
                    self.items.push(Arc::new(value));
                    self.state.splice(index..index, 1);
                }
                VectorDiff::Reset { values } => {
                    let previous = self.state.logical_scroll_top();
                    let anchor_id = self.items.get(previous.item_ix).map(|row| row.id.clone());
                    self.items = values.into_iter().map(Arc::new).collect();
                    self.state.reset(self.items.len());
                    if let Some(index) = anchor_id.and_then(|id| {
                        self.items.iter().position(|row| row.id == id)
                    }) {
                        self.state.scroll_to(ListOffset {
                            item_ix: index,
                            offset_in_item: previous.offset_in_item,
                        });
                    }
                }
            }
        }
    }
}

impl Render for ProofList {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let items = self.items.clone();
        list(self.state.clone(), move |index, _, _| {
            div()
                .h(px(40.))
                .child(items[index].title.clone())
                .into_any_element()
        })
    }
}

#[cfg(test)]
mod tests {
    use gpui::{
        AppContext as _, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, size,
    };

    use super::*;

    #[gpui_kit::test]
    fn prepend_and_edit_splice_without_moving_the_visible_identity(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (window, view) = cx
            .update(|cx| {
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: Point::default(),
                            size: size(px(300.), px(60.)),
                        })),
                        ..WindowOptions::default()
                    },
                    cx,
                    |_, cx| cx.new(|_| ProofList::new()),
                )
            })
            .unwrap();

        let fixture = batches();
        cx.update(|cx| view.update(cx, |list, cx| {
            list.apply(fixture[0].clone());
            cx.notify();
        }));
        cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
        cx.update(|cx| view.update(cx, |list, _| {
            list.state.scroll_to(ListOffset { item_ix: 1, offset_in_item: px(8.) });
            let original = Arc::clone(&list.items[1]);
            list.apply(fixture[1].clone());
            assert_eq!(list.state.item_count(), 4);
            assert_eq!(list.state.logical_scroll_top().item_ix, 2);
            assert_eq!(list.state.logical_scroll_top().offset_in_item, px(8.));
            assert!(Arc::ptr_eq(&original, &list.items[2]));
            assert_eq!(list.items[0].title, "Alpha edited");
        }));
    }

    #[gpui_kit::test]
    fn reset_reanchors_matching_id_and_drops_a_missing_anchor(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let list = cx.update(|cx| cx.new(|_| ProofList::new()));
        let fixture = batches();
        cx.update(|cx| list.update(cx, |list, _| {
            list.apply(fixture[0].clone());
            list.state.scroll_to(ListOffset { item_ix: 1, offset_in_item: px(6.) });
            list.apply(vec![VectorDiff::Reset {
                values: vec![
                    ProofRow { id: "x".into(), title: "Extra".into() },
                    ProofRow { id: "c".into(), title: "Charlie".into() },
                ],
            }]);
            assert_eq!(list.state.item_count(), 2);
            assert_eq!(list.state.logical_scroll_top().item_ix, 1);
            assert_eq!(list.state.logical_scroll_top().offset_in_item, px(6.));
            list.apply(fixture[2].clone());
            assert_eq!(list.state.item_count(), 1);
            assert_eq!(list.state.logical_scroll_top().item_ix, 0);
            assert_eq!(list.items[0].id, "z");
        }));
    }
}
