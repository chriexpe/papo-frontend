//! Debug-only, bounded local cursor trace. No message text or credentials.
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};

const FILE_LIMIT: usize = 128 * 1024;
pub(super) struct ReadingDebug {
    path: PathBuf,
    last: Option<Value>,
}
impl ReadingDebug {
    pub(super) fn new(database: &Path, key: &str) -> Self {
        Self {
            path: database.with_file_name(format!("reading-debug-{key}.jsonl")),
            last: None,
        }
    }
    pub(super) fn record(&mut self, source: &str, before: Value, after: Value, detail: Value) {
        if before == after && detail.get("type").and_then(Value::as_str) != Some("viewport") {
            return;
        }
        let mut record =
            json!({"source": source, "before": before, "after": after, "detail": detail});
        if self.last.as_ref() == Some(&record) {
            return;
        }
        self.last = Some(record.clone());
        record["at"] = json!(papo_core::cache::now_millis());
        let Ok(mut bytes) = serde_json::to_vec(&record) else {
            return;
        };
        bytes.push(b'\n');
        if bytes.len() > FILE_LIMIT {
            return;
        }
        if std::fs::metadata(&self.path)
            .map(|m| m.len() as usize)
            .unwrap_or(0)
            + bytes.len()
            > FILE_LIMIT
        {
            // At most two 128 KiB files per configured server, including restarts.
            let _ = std::fs::rename(&self.path, self.path.with_extension("previous.jsonl"));
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = file.write_all(&bytes);
        }
    }
}

#[cfg(all(debug_assertions, target_os = "android"))]
impl super::Client {
    pub(super) fn reading_debug_state(&self) -> Value {
        let store = &self.runtime.store;
        let mut value = store.reading_diagnostics(&store.selected_channel);
        value["navigationTarget"] = json!(self.navigation_target);
        value["seekingCatchUp"] = json!(self.catching_up.is_some());
        value
    }
    pub(super) fn reading_debug_action(&self, action: &super::Action) -> Value {
        use super::Action;
        match action {
            Action::Visible {
                channel_id,
                ids,
                at_latest,
                ..
            } => {
                let store = &self.runtime.store;
                json!({"type":"viewport", "channel":channel_id, "atLatest":at_latest,
                    "visible":ids.iter().take(64).map(|id| json!({"id":id,
                        "key":store.message(id).map(|m|(m.at.with_timezone(&chrono::Utc), &m.id)),
                        "own":store.message(id).map(|m|m.author_id==store.me)})).collect::<Vec<_>>()})
            }
            Action::BreakReadSpan { channel_id } => {
                json!({"type":"break_span","channel":channel_id})
            }
            Action::CatchUp { channel_id, .. } => json!({"type":"catch_up","channel":channel_id}),
            Action::ClearCatchUp { channel_id, .. } => {
                json!({"type":"clear_catch_up","channel":channel_id})
            }
            Action::Latest { channel_id } => json!({"type":"latest","channel":channel_id}),
            Action::JumpToMessage {
                channel_id,
                message_id,
            } => json!({"type":"jump","channel":channel_id,"id":message_id}),
            Action::SelectChannel { channel_id } => json!({"type":"select","channel":channel_id}),
            _ => json!({"type":"other"}),
        }
    }
    pub(super) fn reading_debug_update(update: &super::Update) -> Value {
        use papo_core::api::ws::Event;
        match update {
            super::Update::Event(event) => match event.as_ref() {
                Event::Message(m) => {
                    json!({"type":"incoming","id":m.id,"channel":m.channel_id,"at":m.created_at})
                }
                Event::MessageEmbedsUpdated {
                    channel_id,
                    message_id,
                    ..
                } => json!({"type":"embeds","channel":channel_id,"id":message_id}),
                _ => json!({"type":"event"}),
            },
            super::Update::Connection(state) => {
                json!({"type":"connection","state":format!("{state:?}")})
            }
            _ => json!({"type":"update"}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trace_rotates_at_a_fixed_size_and_ignores_identical_snapshots() {
        let dir = std::env::temp_dir().join(format!(
            "papo-reading-trace-{}-{}",
            std::process::id(),
            papo_core::cache::now_millis()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut trace = ReadingDebug::new(&dir.join("messages.db"), "fixture");
        trace.record("poll", json!({}), json!({}), json!({}));
        assert!(!trace.path.exists());
        for n in 0..1000 {
            trace.record(
                "viewport",
                json!({"frontier":n}),
                json!({"frontier":n+1}),
                json!({"visible":"x".repeat(1024)}),
            );
        }
        let previous = trace.path.with_extension("previous.jsonl");
        for path in [&trace.path, &previous] {
            assert!(std::fs::metadata(path).unwrap().len() <= FILE_LIMIT as u64);
            for line in std::fs::read_to_string(path).unwrap().lines() {
                assert!(serde_json::from_str::<Value>(line).is_ok());
            }
        }
        assert!(
            std::fs::read_to_string(&trace.path)
                .unwrap()
                .contains("1000")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
