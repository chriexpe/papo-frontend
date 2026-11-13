//! Owns bounded REST/gateway-upgrade dispatch and response encoding; leaves fixture state and socket lifetime outside.

use std::{convert::Infallible, sync::Arc, time::Duration};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{Request, Response, body::Incoming, header};
use tokio::sync::{Mutex, mpsc};

use crate::{
    Config,
    auth::cookie_value,
    gateway,
    response::{Reply, problem},
    server::{MAX_BODY_BYTES, State, take_step},
};

type HttpResponse = Response<Full<Bytes>>;

pub(crate) async fn handle(
    request: Request<Incoming>,
    config: Arc<Config>,
    state: Arc<Mutex<State>>,
    upgrades: mpsc::Sender<(hyper::upgrade::OnUpgrade, String)>,
) -> Result<HttpResponse, Infallible> {
    let method = request.method().as_str().to_owned();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();
    let cookie = cookie_value(request.headers()).map(str::to_owned);
    let media_headers = request.headers().clone();
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let (step, base_url, _upload_guard) = observe(&mut *state.lock().await, &method, &path);
    let encode = |reply| encode(reply, &base_url);
    if let Some(step) = step {
        tokio::time::sleep(Duration::from_millis(step.delay_ms)).await;
        if let Some(failure) = step.failure {
            return Ok(encode(problem(failure.status, &failure.slug, &path)));
        }
    }
    if method == "GET" && path == "/ws" {
        return upgrade(request, &config, &state, &upgrades, &base_url).await;
    }
    let request_hold = take_hold(&mut *state.lock().await, &method, &path, true);
    if let Some(hold) = request_hold {
        hold.wait().await;
    }
    let Ok(body) = Limited::new(request.into_body(), MAX_BODY_BYTES)
        .collect()
        .await
    else {
        return Ok(encode(problem(413, "invalid-param", &path)));
    };
    let body = body.to_bytes();
    let (reply, hold) = {
        let mut state = state.lock().await;
        let user_id = state.auth.user_id(cookie.as_deref()).map(str::to_owned);
        if method == "POST" && path == "/messages" {
            state.message_body = Some(body.clone());
        }
        let reply = route(
            &mut state,
            &config,
            &method,
            &path,
            &query,
            user_id.as_deref(),
            &media_headers,
            &content_type,
            cookie.as_deref(),
            &body,
        );
        let hold = state
            .holds
            .iter()
            .position(|hold| !hold.before_body && hold.method == method && hold.path == path)
            .map(|index| state.holds.remove(index));
        (reply, hold)
    };
    if let Some(hold) = hold {
        hold.wait().await;
    }
    Ok(encode(reply))
}

fn auth_reply(
    state: &mut State,
    config: &Config,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: &[u8],
) -> Reply {
    let reply = state.auth.handle(config, method, path, cookie, body);
    if path == "/auth/register"
        && reply.status == 201
        && let Some(value) = reply.body.as_ref()
        && let (Some(id), Some(username)) = (value["id"].as_str(), value["username"].as_str())
    {
        state.resources.register(id, username);
        state.presence.user_registered(id);
    }
    reply
}

fn encode(mut reply: Reply, base_url: &str) -> HttpResponse {
    if reply.problem
        && let Some(body) = reply.body.as_mut()
        && let Some(slug) = body["type"].as_str()
    {
        body["type"] = format!("{base_url}{slug}").into();
    }
    let has_body = reply.body.is_some();
    let body = reply.encoded.take().unwrap_or_else(|| {
        reply
            .body
            .map_or_else(Vec::new, |value| value.to_string().into_bytes())
    });
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = hyper::StatusCode::from_u16(reply.status)
        .unwrap_or(hyper::StatusCode::INTERNAL_SERVER_ERROR);
    if has_body {
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static(if reply.problem {
                "application/problem+json"
            } else {
                "application/json"
            }),
        );
    }
    if let Some(cookie) = reply
        .cookie
        .and_then(|value| header::HeaderValue::from_str(&value).ok())
    {
        response.headers_mut().insert(header::SET_COOKIE, cookie);
    }
    for (name, value) in reply.headers {
        if let (Ok(name), Ok(value)) = (
            header::HeaderName::from_bytes(name.as_bytes()),
            header::HeaderValue::from_str(&value),
        ) {
            response.headers_mut().insert(name, value);
        }
    }
    response
}

async fn upgrade(
    mut request: Request<Incoming>,
    config: &Config,
    state: &Mutex<State>,
    upgrades: &mpsc::Sender<(hyper::upgrade::OnUpgrade, String)>,
    base_url: &str,
) -> Result<HttpResponse, Infallible> {
    let cookie = cookie_value(request.headers()).map(str::to_owned);
    let path = request.uri().path().to_owned();
    let encode = |reply| encode(reply, base_url);
    let state = state.lock().await;
    if !state.gateway_accepting {
        return Ok(encode(problem(503, "unavailable", &path)));
    }
    let user_id = state.auth.user_id(cookie.as_deref()).map(str::to_owned);
    let Some(user_id) = user_id else {
        return Ok(encode(problem(401, "unauthorized", &path)));
    };
    if !gateway::origin_allowed(request.headers(), &config.allowed_origins) {
        return Ok(encode(problem(403, "ws-upgrade", &path)));
    }
    let Ok(response) = gateway::upgrade_response(&request) else {
        return Ok(encode(problem(400, "ws-upgrade", &path)));
    };
    let pending = hyper::upgrade::on(&mut request);
    if upgrades.try_send((pending, user_id)).is_err() {
        return Ok(encode(problem(500, "internal", &path)));
    }
    Ok(response)
}

fn take_hold(
    state: &mut State,
    method: &str,
    path: &str,
    before_body: bool,
) -> Option<crate::rest_hold::PendingHold> {
    state
        .holds
        .iter()
        .position(|hold| {
            hold.before_body == before_body && hold.method == method && hold.path == path
        })
        .map(|index| state.holds.remove(index))
}

fn observe(
    state: &mut State,
    method: &str,
    path: &str,
) -> (
    Option<crate::RestStep>,
    String,
    Option<crate::read_requests::UploadGuard>,
) {
    state.member_requests.record(method, path);
    if crate::message_requests::relevant(method, path) {
        state.outgoing.requests = state.outgoing.requests.saturating_add(1);
    }
    (
        take_step(state, method, path),
        state.base_url.clone(),
        state.read_requests.record(method, path),
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "fixture HTTP request parts are borrowed only during routing"
)]
fn route(
    state: &mut State,
    config: &Config,
    method: &str,
    path: &str,
    query: &str,
    user_id: Option<&str>,
    media_headers: &hyper::HeaderMap,
    content_type: &str,
    cookie: Option<&str>,
    body: &[u8],
) -> Reply {
    if let Some(reply) =
        crate::notification_search::handle(state, method, path, query, user_id, body)
    {
        reply
    } else if let Some(reply) =
        crate::encoded_media::handle(state, method, path, user_id, media_headers)
    {
        reply
    } else if let Some(reply) =
        crate::message_requests::handle(state, method, path, user_id, content_type, body)
    {
        reply
    } else if let Some(reply) = crate::read_marks::handle(state, method, path, user_id, body) {
        reply
    } else if let Some(reply) = crate::channel_list::handle(state, method, path, user_id) {
        reply
    } else if let Some(reply) = crate::history_http::handle(state, method, path, query, user_id) {
        reply
    } else if let Some(reply) = state
        .resources
        .handle(config, method, path, query, user_id, body)
    {
        reply
    } else {
        auth_reply(state, config, method, path, cookie, body)
    }
}
