//! Owns one server's cookie-authenticated REST transport; session persistence and retry belong to the SDK.

use crate::{
    Error, HistoryQuery, Uuid,
    routes::{self, Route},
    wire::{
        Channel, ChannelList, CreateChannelRequest, CreateServerRequest, CreateServerResponse,
        Message, MessageListResponse, Problem, Server, UpdateChannelRequest, UpdateMessageRequest,
    },
};
use reqwest::{
    Method, RequestBuilder, Response,
    cookie::{CookieStore, Jar},
};
use serde::{Serialize, de::DeserializeOwned};
use std::sync::Arc;

/// An in-memory cookie transport scoped to exactly one validated backend address.
/// It follows no redirects, persists no cookies and performs no policy retries.
#[derive(Clone)]
pub struct RestClient {
    client: reqwest::Client,
    anonymous: reqwest::Client,
    cookies: Arc<Jar>,
    base: url::Url,
    insecure_loopback: bool,
}

impl RestClient {
    /// Creates a transport using normal Secure-cookie rules.
    /// # Errors
    /// Returns a sanitized address or client-construction failure.
    pub fn new(address: &str) -> Result<Self, Error> {
        Self::create(address, false)
    }

    /// Allows Secure cookies over HTTP solely for literal loopback integration servers.
    /// # Errors
    /// Rejects non-loopback addresses and invalid base URLs.
    pub fn loopback(address: &str) -> Result<Self, Error> {
        Self::create(address, true)
    }

    fn create(address: &str, insecure_loopback: bool) -> Result<Self, Error> {
        let mut base = url::Url::parse(address).map_err(|_| Error::Address)?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || (insecure_loopback
                && !base.host_str().is_some_and(|host| {
                    host.trim_matches(['[', ']'])
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
                }))
        {
            return Err(Error::Address);
        }
        base.set_path(&format!("{}/", base.path().trim_end_matches('/')));
        let cookies = Arc::new(Jar::default());
        let client = reqwest::Client::builder()
            .cookie_provider(Arc::clone(&cookies))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map_err(|_| Error::Transport)?;
        let anonymous = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map_err(|_| Error::Transport)?;
        Ok(Self {
            client,
            anonymous,
            cookies,
            base,
            insecure_loopback,
        })
    }

    pub(crate) fn endpoint(&self, path: &str) -> Result<url::Url, Error> {
        self.base
            .join(path.trim_start_matches('/'))
            .map_err(|_| Error::Address)
    }

    pub(crate) fn gateway_cookie(&self) -> Option<reqwest::header::HeaderValue> {
        self.cookies.cookies(&self.base)
    }

    pub(crate) fn request(&self, method: Method, path: &str) -> Result<RequestBuilder, Error> {
        Ok(self.client.request(method, self.endpoint(path)?))
    }

    pub(crate) fn anonymous_request(
        &self,
        method: Method,
        path: &str,
    ) -> Result<RequestBuilder, Error> {
        Ok(self.anonymous.request(method, self.endpoint(path)?))
    }

    pub(crate) async fn response(&self, request: RequestBuilder) -> Result<Response, Error> {
        self.response_inner(request, true).await
    }

    pub(crate) async fn anonymous_response(
        &self,
        request: RequestBuilder,
    ) -> Result<Response, Error> {
        self.response_inner(request, false).await
    }

    async fn response_inner(
        &self,
        request: RequestBuilder,
        authenticated: bool,
    ) -> Result<Response, Error> {
        let response = request.send().await.map_err(|_| Error::Transport)?;
        if authenticated && self.insecure_loopback {
            for cookie in response.headers().get_all(reqwest::header::SET_COOKIE) {
                if let Ok(cookie) = cookie.to_str() {
                    let cookie = cookie
                        .split(';')
                        .filter(|part| !part.trim().eq_ignore_ascii_case("secure"))
                        .collect::<Vec<_>>()
                        .join(";");
                    self.cookies.add_cookie_str(&cookie, &self.base);
                }
            }
        }
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let status = status.as_u16();
        match response.json::<Problem>().await {
            Ok(problem) => Err(Error::Problem {
                status,
                problem: Box::new(problem),
            }),
            Err(_) => Err(Error::Http { status }),
        }
    }

    pub(crate) async fn json<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
    ) -> Result<T, Error> {
        self.response(request)
            .await?
            .json()
            .await
            .map_err(|_| Error::Decode)
    }

    pub(crate) async fn anonymous_json<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
    ) -> Result<T, Error> {
        self.anonymous_response(request)
            .await?
            .json()
            .await
            .map_err(|_| Error::Decode)
    }

    pub(crate) fn route_request(
        &self,
        route: &Route,
        values: &[String],
    ) -> Result<RequestBuilder, Error> {
        self.request(route.method.clone(), &route.url(values))
    }

    pub(crate) fn anonymous_route_request(
        &self,
        route: &Route,
        values: &[String],
    ) -> Result<RequestBuilder, Error> {
        self.anonymous_request(route.method.clone(), &route.url(values))
    }

    pub(crate) async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        route: &Route,
        values: &[String],
        body: &B,
    ) -> Result<T, Error> {
        self.json(self.route_request(route, values)?.json(body))
            .await
    }

    /// Reads public discovery metadata.
    /// # Errors
    /// Returns transport, problem or decoding failures.
    pub async fn server(&self) -> Result<Server, Error> {
        self.json(self.route_request(&routes::GET_SERVER, &[])?)
            .await
    }
    /// Creates the initial server.
    /// # Errors
    /// Returns transport, problem or decoding failures.
    pub async fn create_server(
        &self,
        request: &CreateServerRequest,
    ) -> Result<CreateServerResponse, Error> {
        self.post(&routes::POST_SERVER, &[], request).await
    }
    /// Lists authorized channels without advancing read positions.
    /// # Errors
    /// Returns transport, problem or decoding failures.
    pub async fn channels(&self) -> Result<ChannelList, Error> {
        self.json(self.route_request(&routes::GET_CHANNELS, &[])?)
            .await
    }
    /// Creates a channel using the backend's supported create fields.
    /// # Errors
    /// Returns transport, problem or decoding failures.
    pub async fn create_channel(&self, request: &CreateChannelRequest) -> Result<Channel, Error> {
        self.post(&routes::POST_CHANNELS, &[], request).await
    }
    /// Updates a channel.
    /// # Errors
    /// Returns transport, problem or decoding failures.
    pub async fn update_channel(
        &self,
        id: Uuid,
        request: &UpdateChannelRequest,
    ) -> Result<Channel, Error> {
        self.json(
            self.route_request(&routes::PUT_CHANNELS_CHANNEL_ID, &[id.to_string()])?
                .json(request),
        )
        .await
    }
    /// Deletes a channel.
    /// # Errors
    /// Returns transport or backend rejection.
    pub async fn delete_channel(&self, id: Uuid) -> Result<(), Error> {
        self.response(self.route_request(&routes::DELETE_CHANNELS_CHANNEL_ID, &[id.to_string()])?)
            .await
            .map(|_| ())
    }
    /// Fetches one page with a strict, exact tuple cursor. Never marks messages read.
    /// # Errors
    /// Returns transport, problem or decoding failures.
    pub async fn messages(
        &self,
        channel: Uuid,
        query: &HistoryQuery<'_>,
    ) -> Result<MessageListResponse, Error> {
        self.json(
            self.route_request(
                &routes::GET_CHANNELS_CHANNEL_ID_MESSAGES,
                &[channel.to_string()],
            )?
            .query(&query.parameters()),
        )
        .await
    }
    /// Sends a multipart message once. The caller owns ambiguous-send policy.
    /// # Errors
    /// Returns transport, problem or decoding failures.
    pub async fn create_message(&self, request: &MessageUpload<'_>) -> Result<Message, Error> {
        let mut form =
            reqwest::multipart::Form::new().text("channel_id", request.channel_id.to_owned());
        for (key, value) in [
            ("content", request.content),
            ("reply_to", request.reply_to),
            ("embeds", request.embeds),
        ] {
            if let Some(value) = value {
                form = form.text(key, value.to_owned());
            }
        }
        for attachment in request.attachments {
            form = form.part(
                "attachments",
                reqwest::multipart::Part::bytes(attachment.bytes.to_vec())
                    .file_name(attachment.filename.to_owned()),
            );
        }
        self.json(
            self.route_request(&routes::POST_MESSAGES, &[])?
                .multipart(form),
        )
        .await
    }
    /// Edits a message once.
    /// # Errors
    /// Returns transport, problem or decoding failures.
    pub async fn update_message(
        &self,
        id: Uuid,
        request: &UpdateMessageRequest,
    ) -> Result<Message, Error> {
        self.json(
            self.route_request(&routes::PUT_MESSAGES_MESSAGE_ID, &[id.to_string()])?
                .json(request),
        )
        .await
    }
    /// Deletes a message once.
    /// # Errors
    /// Returns transport or backend rejection.
    pub async fn delete_message(&self, id: Uuid) -> Result<(), Error> {
        self.response(self.route_request(&routes::DELETE_MESSAGES_MESSAGE_ID, &[id.to_string()])?)
            .await
            .map(|_| ())
    }
}

/// A multipart message upload; each attachment retains its caller-selected filename.
pub struct MessageUpload<'a> {
    /// Target channel identifier.
    pub channel_id: &'a str,
    /// Optional message text.
    pub content: Option<&'a str>,
    /// Optional identifier of the message being replied to.
    pub reply_to: Option<&'a str>,
    /// Optional JSON-encoded embed array.
    pub embeds: Option<&'a str>,
    /// Attachments in their intended order.
    pub attachments: &'a [MessageAttachment<'a>],
}

/// One multipart attachment, including its backend-visible original filename.
pub struct MessageAttachment<'a> {
    /// Filename supplied in the multipart content disposition.
    pub filename: &'a str,
    /// Unmodified file contents.
    pub bytes: &'a [u8],
}
