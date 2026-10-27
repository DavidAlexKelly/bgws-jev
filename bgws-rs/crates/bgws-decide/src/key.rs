//! API keys, held by the host (RUST_ENGINE.md §10.2: "keys are kept by the
//! host, not compiled into the UI"). An [`ApiKey`] never prints: `Debug`
//! shows `ApiKey(***)` and there is no `Display`, so a key cannot end up in a
//! log line, an error or a rationale by accident.

use std::fmt;

#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(key: impl Into<String>) -> Option<ApiKey> {
        let k = key.into();
        (!k.trim().is_empty()).then_some(ApiKey(k))
    }
    /// From an environment variable, if set and not empty.
    pub fn from_env(var: &str) -> Option<ApiKey> {
        std::env::var(var).ok().and_then(ApiKey::new)
    }
    /// The secret, for a request header only.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(***)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn never_prints() {
        let k = ApiKey::new("sk-secret").unwrap();
        assert_eq!(format!("{k:?}"), "ApiKey(***)");
        assert!(ApiKey::new("  ").is_none());
        assert_eq!(k.expose(), "sk-secret");
    }
}
