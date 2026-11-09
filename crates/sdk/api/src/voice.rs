//! Owns ICE server configuration retrieval; voice signaling belongs to the gateway.

use crate::{Error, RestClient, routes, wire::ICEServersResponse};

impl RestClient {
    /// Fetches the authenticated account's ICE server configuration.
    ///
    /// The response may contain short-lived TURN credentials; do not log it.
    /// # Errors
    /// Returns transport, backend rejection or response decoding failures.
    pub async fn ice_servers(&self) -> Result<ICEServersResponse, Error> {
        self.json(self.route_request(&routes::GET_VOICE_ICE_SERVERS, &[])?)
            .await
    }
}

#[cfg(test)]
mod tests {
    //! Verifies the generated ICE wire shape without network or secrets.

    use super::*;
    use serde_json::json;

    #[test]
    fn ice_servers_decode_the_backend_wrapper() {
        let result: ICEServersResponse = serde_json::from_value(json!({
            "ice_servers": [{"urls": []}]
        }))
        .unwrap();
        let servers = result.ice_servers.unwrap();
        assert_eq!(servers.len(), 1);
        assert!(servers[0].urls.as_ref().is_some_and(Vec::is_empty));
        assert!(servers[0].username.is_none());
        assert!(servers[0].credential.is_none());
    }
}
