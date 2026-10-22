//! Recovery links identify a token and, optionally, a backend. A hint never
//! authorizes sending credentials: callers must compare it to a selected server.
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RecoveryLinkError {
    #[error("invalid_recovery_link")]
    InvalidLink,
    #[error("invalid_server_address")]
    InvalidServer,
    #[error("recovery_server_mismatch")]
    WrongServer,
}

#[derive(Clone, PartialEq, Eq)]
pub struct RecoveryLink {
    pub token: String,
    pub server: Option<String>,
}

pub fn server_address(raw: &str) -> Result<String, RecoveryLinkError> {
    let url = Url::parse(raw.trim()).map_err(|_| RecoveryLinkError::InvalidServer)?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none()
        || !url.username().is_empty() || url.password().is_some()
        || url.query().is_some() || url.fragment().is_some()
    {
        return Err(RecoveryLinkError::InvalidServer);
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

pub fn parse(raw: &str) -> Result<RecoveryLink, RecoveryLinkError> {
    let invalid = RecoveryLinkError::InvalidLink;
    let link = Url::parse(raw.trim()).map_err(|_| invalid)?;
    if !link.username().is_empty() || link.password().is_some() || link.fragment().is_some() {
        return Err(invalid);
    }
    let token = if link.scheme() == "https" && link.host_str() == Some("papo.cyberasilo.online")
        && link.port_or_known_default() == Some(443)
    {
        link.path().strip_prefix("/passwordchange/").ok_or(invalid)?
    } else if link.scheme() == "papo-reset" && link.host_str() == Some("passwordchange") && link.port().is_none() {
        link.path().strip_prefix('/').ok_or(invalid)?
    } else {
        return Err(invalid);
    };
    // Backend-generated tokens are URL-safe base64. Reject extra paths,
    // percent escapes and ambiguous queries rather than interpreting them.
    let token = token.trim_end_matches('/');
    if token.is_empty() || token.len() > 256 || !token.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_')) {
        return Err(invalid);
    }
    let mut hints = link.query_pairs().filter(|(k, _)| k == "server");
    let server = hints.next().map(|(_, v)| server_address(&v)).transpose()?;
    if hints.next().is_some() { return Err(invalid); }
    Ok(RecoveryLink { token: token.to_owned(), server })
}

impl RecoveryLink {
    pub fn for_server(&self, selected: &str) -> Result<(), RecoveryLinkError> {
        let selected = server_address(selected)?;
        if self.server.as_ref().is_some_and(|hint| hint != &selected) {
            return Err(RecoveryLinkError::WrongServer);
        }
        Ok(())
    }
}

pub fn with_server(raw: &str, server: &str) -> Result<String, RecoveryLinkError> {
    parse(raw)?;
    let server = server_address(server)?;
    let mut link = Url::parse(raw).map_err(|_| RecoveryLinkError::InvalidLink)?;
    let others: Vec<_> = link.query_pairs().filter(|(k, _)| k != "server").map(|(k,v)| (k.into_owned(),v.into_owned())).collect();
    link.set_query(None);
    link.query_pairs_mut().extend_pairs(others).append_pair("server", &server);
    Ok(link.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_both_supported_links_and_explicitly_selected_legacy_server() {
        for raw in ["https://papo.cyberasilo.online/passwordchange/ab_CD-12", "papo-reset://passwordchange/ab_CD-12"] {
            let p = parse(raw).unwrap();
            assert_eq!(p.token, "ab_CD-12");
            assert!(p.for_server("https://example.org").is_ok());
            let scoped = with_server(raw,"https://EXAMPLE.org/").unwrap();
            let p = parse(&scoped).unwrap();
            assert!(p.for_server("https://example.org").is_ok());
            for other in ["https://other.org", "http://example.org", "https://example.org:8443", "https://example.org/path"] {
                assert!(p.for_server(other).is_err());
            }
        }
    }
    #[test]
    fn rejects_ambiguous_and_foreign_links() {
        for raw in ["https://evil.org/passwordchange/abc", "https://papo.cyberasilo.online.evil.org/passwordchange/abc", "https://u@papo.cyberasilo.online/passwordchange/abc", "https://papo.cyberasilo.online:444/passwordchange/abc", "papo-reset://passwordchange/", "papo-reset://passwordchange/a/b", "papo-reset://passwordchange/abc%2Fdef", "papo-reset://passwordchange/abc?server=https://a&server=https://b", "papo-reset://passwordchange/abc?server=https://u:p@a", "papo-reset://passwordchange/abc#hidden"] {
            assert!(parse(raw).is_err(), "{raw}");
        }
    }
}
