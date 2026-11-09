//! Owns the unauthenticated health route; transport remains in the REST client.

use crate::{Error, RestClient, routes};
impl RestClient {
    /// Reads the unauthenticated health endpoint as plain text without sending cookies.
    /// # Errors
    /// Returns a transport, backend rejection or response decoding failure.
    pub async fn health(&self) -> Result<String, Error> {
        self.anonymous_response(self.anonymous_route_request(&routes::GET_HEALTH, &[])?)
            .await?
            .text()
            .await
            .map_err(|_| Error::Decode)
    }
}
