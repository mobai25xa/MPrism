//! AuthOptions helpers for request construction.
//!
//! Semantics follow `docs/sources/model-protocol-sdk.md` §3.1.

use crate::error::{ProtocolError, ProtocolErrorKind};
use crate::types::{AuthOptions, ProviderEndpoint};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use url::Url;

/// Merge `extra_headers` into an existing forced header map.
///
/// Forced / already-present headers win; conflicting extra headers are skipped.
/// Invalid header names/values yield `InvalidConfiguration`.
/// Header **values** must never be copied into [`ProtocolError::message`].
pub fn merge_extra_headers(
    headers: &mut HeaderMap,
    auth: &AuthOptions,
) -> Result<(), ProtocolError> {
    for (name, value) in &auth.extra_headers {
        let name = name.trim();
        if name.is_empty() {
            return Err(ProtocolError::new(
                ProtocolErrorKind::InvalidConfiguration,
                "extra_headers 名称不能为空",
            ));
        }
        if name.chars().any(|c| c == '\r' || c == '\n') {
            return Err(ProtocolError::new(
                ProtocolErrorKind::InvalidConfiguration,
                "extra_headers 名称不能包含 CR/LF",
            ));
        }
        if value.chars().any(|c| c == '\r' || c == '\n') {
            return Err(ProtocolError::new(
                ProtocolErrorKind::InvalidConfiguration,
                "extra_headers 值不能包含 CR/LF",
            ));
        }
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
            ProtocolError::new(
                ProtocolErrorKind::InvalidConfiguration,
                "extra_headers 名称无效",
            )
        })?;
        if headers.contains_key(&header_name) {
            // Forced headers win; skip conflicting extras.
            continue;
        }
        let header_value = HeaderValue::from_str(value).map_err(|_| {
            ProtocolError::new(
                ProtocolErrorKind::InvalidConfiguration,
                "extra_headers 值无效",
            )
        })?;
        headers.insert(header_name, header_value);
    }
    Ok(())
}

/// Append API key as a query parameter when configured and key is non-empty.
pub fn apply_api_key_query(mut url: Url, endpoint: &ProviderEndpoint) -> Url {
    let Some(param) = endpoint
        .auth
        .api_key_query_param
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return url;
    };
    if endpoint.api_key.is_empty() {
        return url;
    }
    url.query_pairs_mut()
        .append_pair(param, endpoint.api_key.expose_secret());
    url
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::SecretString;
    use crate::types::{normalize_base_url, ProtocolKind};
    use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};

    #[test]
    fn forced_headers_win_over_extra() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer forced"));
        let auth = AuthOptions {
            extra_headers: vec![
                ("Content-Type".into(), "text/plain".into()),
                ("X-Custom".into(), "yes".into()),
            ],
            api_key_query_param: None,
        };
        merge_extra_headers(&mut headers, &auth).unwrap();
        assert_eq!(
            headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok()),
            Some("application/json")
        );
        assert_eq!(
            headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()),
            Some("Bearer forced")
        );
        assert_eq!(
            headers.get("x-custom").and_then(|v| v.to_str().ok()),
            Some("yes")
        );
    }

    #[test]
    fn rejects_crlf_in_header_name() {
        let mut headers = HeaderMap::new();
        let auth = AuthOptions {
            extra_headers: vec![("X-Bad\nName".into(), "v".into())],
            api_key_query_param: None,
        };
        let err = merge_extra_headers(&mut headers, &auth).unwrap_err();
        assert_eq!(err.kind, ProtocolErrorKind::InvalidConfiguration);
        assert!(!err.message.contains('\n'));
    }

    #[test]
    fn api_key_query_only_when_configured_and_non_empty() {
        let base = normalize_base_url("https://api.example.com/v1").unwrap();
        let with = ProviderEndpoint {
            protocol: ProtocolKind::OpenAiChatCompletions,
            base_url: base.clone(),
            api_key: SecretString::new("sk-test"),
            auth: AuthOptions {
                extra_headers: vec![],
                api_key_query_param: Some("key".into()),
            },
        };
        let url = apply_api_key_query(base.clone(), &with);
        assert_eq!(url.query(), Some("key=sk-test"));

        let empty_key = ProviderEndpoint {
            protocol: ProtocolKind::OpenAiChatCompletions,
            base_url: base.clone(),
            api_key: SecretString::new(""),
            auth: AuthOptions {
                extra_headers: vec![],
                api_key_query_param: Some("key".into()),
            },
        };
        let url = apply_api_key_query(base.clone(), &empty_key);
        assert_eq!(url.query(), None);

        let no_param = ProviderEndpoint {
            protocol: ProtocolKind::OpenAiChatCompletions,
            base_url: base.clone(),
            api_key: SecretString::new("sk-test"),
            auth: AuthOptions::default(),
        };
        let url = apply_api_key_query(base, &no_param);
        assert_eq!(url.query(), None);
    }
}
