//! Chunk identity in the coverage simulation: uniqueness and no reuse checked after every
//! step, and a restart through the stored layout. Coverage itself is checked in `simulation`.

use std::collections::BTreeSet;

use super::super::persisted_layout::{LAYOUT_VERSION, Metadata, encode, layout, partition};
use super::super::persistence::decode;
use super::simulation::Simulation;
use crate::account_cache::history::{HistoryData, HistoryRow, HistoryTable};

impl Simulation {
    /// Every run has its own identity, issued by the set's allocator, and an identity that
    /// left the model (union, retention, an empty-channel proof) never returns, across reloads
    /// too.
    pub(super) fn check_identities(&mut self) {
        let held: BTreeSet<_> = self.model.chunks.iter().map(|chunk| chunk.id).collect();
        assert_eq!(
            held.len(),
            self.model.chunks.len(),
            "two runs share an identity"
        );
        for id in &held {
            assert!(
                self.model.chunk_ids.issued(*id),
                "identity was never issued"
            );
            assert!(
                self.identities_held.contains(id) || !self.identities_seen.contains(id),
                "a retired identity was reused"
            );
        }
        self.identities_seen.extend(held.iter().copied());
        self.identities_held = held;
    }

    /// A restart: the rows a complete persist leaves are decoded into a new model. Connection
    /// evidence is process-local, so the restart is followed by a reconnect.
    pub(super) fn reload(&mut self) {
        let mut rows: Vec<_> = layout(&self.model)
            .unwrap()
            .into_iter()
            .map(|((table, id), payload)| HistoryRow {
                table: partition(table).unwrap(),
                id,
                payload,
            })
            .collect();
        for (id, entry) in &self.model.entries {
            if !entry.deleted {
                rows.push(HistoryRow {
                    table: HistoryTable::Messages,
                    id: *id,
                    payload: encode(entry).unwrap(),
                });
            }
        }
        let metadata = encode(&Metadata {
            layout: LAYOUT_VERSION,
            revision: self.model.revision,
            empty_generation: self.model.empty_generation,
            chunk_high_water: self.model.chunk_ids.high_water(),
            active_chunks: self.model.chunks.iter().map(|chunk| chunk.id).collect(),
        })
        .unwrap();
        let data = HistoryData {
            set_id: 1,
            metadata,
            rows,
        };
        let restored = decode(data).unwrap().expect_current();
        assert_eq!(restored.model.chunks, {
            let mut chunks = self.model.chunks.clone();
            for chunk in &mut chunks {
                chunk.generation = 0;
                chunk.live_generation = 0;
            }
            chunks
        });
        self.model = restored.model;
        self.reconnect(0);
    }
}
