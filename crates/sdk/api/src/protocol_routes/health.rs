//! Owns the unauthenticated health route; transport remains in the REST client.

use crate::{Error, RestClient};
use reqwest::Method;

const HEALTH_PATH: &str = "health";

impl RestClient {
    /// Reads the unauthenticated health endpoint as plain text without sending cookies.
    /// # Errors
    /// Returns a transport, backend rejection or response decoding failure.
    pub async fn health(&self) -> Result<String, Error> {
        self.anonymous_response(self.anonymous_request(Method::GET, HEALTH_PATH)?)
            .await?
            .text()
            .await
            .map_err(|_| Error::Decode)
    }
}

#[cfg(test)]
mod tests {
    //! Verifies the health path without network access.

    use super::*;

    #[test]
    fn health_path_matches_backend_spelling() {
        assert_eq!(HEALTH_PATH, "health");
    }
}
