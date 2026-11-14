//! Explicit operation scripts and resource mutation fixtures; production authorization belongs to Go proofs.

mod administration;
mod personal;

use std::collections::{BTreeMap, VecDeque};

use serde_json::Value;

use crate::{
    Error, TestServer,
    response::{Reply, problem},
    server::State,
};

struct ScriptedResponse {
    method: String,
    path: String,
    status: u16,
    value: Value,
}

/// Bounded in-memory controls, not a substitute for backend permission/parity tests.
#[derive(Default)]
pub(crate) struct Operations {
    scripts: VecDeque<ScriptedResponse>,
    requests: Vec<(String, String, String, Value)>,
    pub(crate) settings: BTreeMap<String, Value>,
    next_id: u64,
}

impl Operations {
    fn id(&mut self) -> String {
        self.next_id = self.next_id.saturating_add(1);
        format!("00000000-0000-4000-9000-{:012x}", self.next_id)
    }
}

impl TestServer {
    /// Queues an explicit HTTP response for an authenticated operation or listing.
    /// Scripts do not mutate resources or simulate backend authorization. Use pinned-Go proofs for parity.
    /// # Errors
    /// Rejects malformed methods/routes/status and scripts beyond the bounded 512-response capacity.
    pub async fn queue_operation_response(
        &self,
        method: &str,
        path: &str,
        status: u16,
        value: Value,
    ) -> Result<(), Error> {
        if hyper::Method::from_bytes(method.as_bytes()).is_err()
            || !path.starts_with('/')
            || !(200..=599).contains(&status)
        {
            return Err(Error::Configuration);
        }
        let mut state = self.state.lock().await;
        if state.operations.scripts.len() >= 512 {
            return Err(Error::Configuration);
        }
        state.operations.scripts.push_back(ScriptedResponse {
            method: method.into(),
            path: path.into(),
            status,
            value,
        });
        Ok(())
    }

    /// Consumes bounded method/path/exact-query/body observations; never logs request data.
    pub async fn take_operation_requests(&self) -> Vec<(String, String, String, Value)> {
        std::mem::take(&mut self.state.lock().await.operations.requests)
    }
}

pub(crate) fn handle(
    state: &mut State,
    method: &str,
    path: &str,
    query: &str,
    user: Option<&str>,
    body: &[u8],
) -> Option<Reply> {
    let scripted = state
        .operations
        .scripts
        .iter()
        .position(|script| script.method == method && script.path == path);
    if scripted.is_none()
        && !personal::relevant(method, path)
        && !administration::relevant(method, path)
    {
        return None;
    }
    let Some(user) = user else {
        return Some(problem(401, "unauthorized", path));
    };
    let value = if body.is_empty() {
        Value::Null
    } else {
        match serde_json::from_slice(body) {
            Ok(value) => value,
            Err(_) if scripted.is_some() => Value::Null,
            Err(_) => return Some(problem(400, "invalid-param", path)),
        }
    };
    if state.operations.requests.len() < 512 {
        state
            .operations
            .requests
            .push((method.into(), path.into(), query.into(), value.clone()));
    }
    if let Some(index) = scripted {
        let script = state.operations.scripts.remove(index)?;
        return Some(if script.status == 204 {
            Reply::empty()
        } else {
            Reply::json(script.status, script.value)
        });
    }
    let parts: Vec<_> = path.trim_matches('/').split('/').collect();
    personal::handle(state, method, path, &parts, user, &value)
        .or_else(|| administration::handle(state, method, path, &parts, &value))
}
