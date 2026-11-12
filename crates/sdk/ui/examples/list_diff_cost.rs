//! Synthetic list-diff work report; excludes canonical history, SQLite, media, rendering and process RSS.
//! Run with `cargo run --locked -p papo-sdk-ui --example list_diff_cost`.
//! Durations are advisory and comparable only with identical toolchain/profile/hardware.

#[path = "../src/list_diff.rs"]
mod list_diff;

use imbl::Vector;
use std::{cell::Cell, time::Instant};

fn main() {
    for rows in [100, 1_000, 10_000] {
        let previous: Vector<_> = (0..rows).map(|id| (id, 0)).collect();
        let mut append = previous.clone();
        append.push_back((rows, 0));
        let prepend = (rows..rows + 100)
            .map(|id| (id, 0))
            .chain(previous.iter().copied())
            .collect();
        let mut edit = previous.clone();
        edit.set(rows / 2, (rows / 2, 1));
        let mut delete = previous.clone();
        delete.remove(rows / 2);
        let reorder = previous.iter().rev().copied().collect();
        for (scenario, next) in [
            ("append", append),
            ("prepend", prepend),
            ("edit", edit),
            ("delete", delete),
            ("reorder", reorder),
        ] {
            let identity_calls = Cell::new(0_u64);
            let started = Instant::now();
            let diffs = list_diff::between(&previous, &next, |item| {
                identity_calls.set(identity_calls.get() + 1);
                item.0
            });
            let elapsed_us = started.elapsed().as_micros();
            let mut replay = previous.clone();
            for diff in &diffs {
                diff.clone().apply(&mut replay);
            }
            assert_eq!(replay, next);
            println!(
                "{{\"schema_version\":1,\"scenario\":\"{scenario}\",\"rows\":{rows},\"identity_calls\":{},\"diffs\":{},\"elapsed_us\":{elapsed_us}}}",
                identity_calls.get(),
                diffs.len()
            );
        }
    }
}
