//! Owns the deterministic media transport scenario; production backend behavior remains authoritative.

use std::{net::Ipv4Addr, sync::Arc};

use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, Response, body::Incoming, header, server::conn::http1, service::service_fn};
use hyper_util::rt::TokioIo;
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};

use crate::Error;

/// The non-body request observations needed by the media transport scenario.
pub struct MediaRequest {
    /// Requested relative path.
    pub path: String,
    /// The synthetic fixture cookie, if present.
    pub cookie: Option<String>,
    /// The requested HTTP byte range, if present.
    pub range: Option<String>,
}

/// A deterministic loopback media scenario with a synthetic login and scripted responses.
pub struct MediaTestServer {
    address: std::net::SocketAddr,
    requests: Arc<Mutex<Vec<MediaRequest>>>,
    task: JoinHandle<()>,
}

impl MediaTestServer {
    /// Starts the isolated media fixture on loopback.
    ///
    /// # Errors
    /// Returns a typed bind error if the listener cannot be opened.
    pub async fn start() -> Result<Self, Error> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| Error::Bind)?;
        let address = listener.local_addr().map_err(|_| Error::Bind)?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observations = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let observations = Arc::clone(&observations);
                let service = service_fn(move |request| reply(request, Arc::clone(&observations)));
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            }
        });
        Ok(Self {
            address,
            requests,
            task,
        })
    }

    /// Returns this fixture's runtime-derived URL.
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// Stops the fixture and returns the observed requests in order.
    pub async fn finish(self) -> Vec<MediaRequest> {
        self.task.abort();
        let _ = self.task.await;
        std::mem::take(&mut *self.requests.lock().await)
    }
}

async fn reply(
    request: Request<Incoming>,
    requests: Arc<Mutex<Vec<MediaRequest>>>,
) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    let mut requests = requests.lock().await;
    let index = requests.len();
    let value = |name| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    requests.push(MediaRequest {
        path: request.uri().path().to_owned(),
        cookie: value(header::COOKIE),
        range: value(header::RANGE),
    });
    let (status, body): (u16, &'static [u8]) = match index {
        0 => (200, b"{}"),
        1 => (206, b"cde"),
        2 => (200, b"full"),
        3 => (200, b"thumb"),
        4 => (416, b""),
        5 => (302, b""),
        6 => (200, b"u"),
        7 => (200, b"w"),
        8 => (200, b"x"),
        _ => (500, b""),
    };
    let mut response = Response::new(Full::new(Bytes::from_static(body)));
    *response.status_mut() =
        hyper::StatusCode::from_u16(status).unwrap_or(hyper::StatusCode::INTERNAL_SERVER_ERROR);
    response.headers_mut().insert(
        header::CONNECTION,
        header::HeaderValue::from_static("close"),
    );
    match index {
        0 => {
            response.headers_mut().insert(
                header::SET_COOKIE,
                header::HeaderValue::from_static("fixture=original; Path=/; Secure"),
            );
        }
        1 => {
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("video/mp4"),
            );
            response.headers_mut().insert(
                header::CONTENT_RANGE,
                header::HeaderValue::from_static("bytes 2-4/10"),
            );
            response.headers_mut().insert(
                header::ACCEPT_RANGES,
                header::HeaderValue::from_static("bytes"),
            );
            response
                .headers_mut()
                .insert(header::ETAG, header::HeaderValue::from_static("\"v1\""));
        }
        3 => {
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("image/webp"),
            );
        }
        4 => {
            response.headers_mut().insert(
                header::CONTENT_RANGE,
                header::HeaderValue::from_static("bytes */10"),
            );
        }
        5 => {
            response.headers_mut().insert(
                header::LOCATION,
                header::HeaderValue::from_static("/unexpected-redirect"),
            );
        }
        6 | 7 => {
            response.headers_mut().insert(
                header::SET_COOKIE,
                header::HeaderValue::from_static("fixture=changed; Path=/; Secure"),
            );
        }
        _ => {}
    }
    Ok(response)
}
