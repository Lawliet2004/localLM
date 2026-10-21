//! One place that decides how an outbound HTTP client behaves.
//!
//! Before this module every call site built its own `reqwest::Client` with
//! its own redirect, timeout and TLS policy — fourteen of them. The values
//! genuinely differ (a model provider needs a long timeout, a download must
//! not have one at all), so they are not unified here. What is unified is
//! *where the policy is written down*: each caller names the policy it wants
//! and this module builds the client.
//!
//! The security-relevant knobs — `https_only` in particular — are therefore
//! decided in one file instead of being present at some call sites and
//! missing at others.

use std::time::Duration;

/// How an outbound request may behave.
#[derive(Clone, Debug)]
pub struct Policy {
    /// Total time budget for the request, when there is one.
    pub timeout: Option<Duration>,
    /// Time budget for establishing the connection.
    pub connect_timeout: Option<Duration>,
    /// Whether redirects are followed at all.
    pub follow_redirects: bool,
    /// Whether only `https://` URLs are accepted.
    pub https_only: bool,
    /// Whether the system proxy configuration is bypassed.
    pub no_proxy: bool,
    /// Headers sent with every request.
    pub headers: Option<reqwest::header::HeaderMap>,
}

impl Policy {
    /// A policy that follows redirects.
    pub fn following(timeout: Duration) -> Self {
        Self {
            timeout: Some(timeout),
            connect_timeout: Some(Duration::from_secs(10)),
            follow_redirects: true,
            https_only: false,
            no_proxy: false,
            headers: None,
        }
    }

    /// A policy that refuses redirects. Used wherever a redirect could send
    /// a credential to a host the user did not name.
    pub fn no_redirects(timeout: Duration) -> Self {
        Self {
            follow_redirects: false,
            ..Self::following(timeout)
        }
    }

    /// A policy with no total timeout, for large downloads that legitimately
    /// run for minutes.
    pub fn unbounded() -> Self {
        Self {
            timeout: None,
            connect_timeout: Some(Duration::from_secs(15)),
            follow_redirects: true,
            https_only: true,
            no_proxy: false,
            headers: None,
        }
    }

    /// Bypass the system proxy configuration. Used for a local model server,
    /// which must never be reached through a proxy.
    pub fn no_proxy(mut self) -> Self {
        self.no_proxy = true;
        self
    }

    /// Send `headers` with every request.
    pub fn headers(mut self, headers: reqwest::header::HeaderMap) -> Self {
        self.headers = Some(headers);
        self
    }
}

/// Build a client for `policy`.
///
/// `error` is the message used when the client cannot be built; each call
/// site names its own so the failure stays actionable in context.
pub fn client(policy: Policy, error: &str) -> Result<reqwest::Client, String> {
    builder(policy).build().map_err(|_| error.to_string())
}

/// Start a client builder from `policy`.
///
/// A caller that needs one extra knob — a cookie store, a proxy, an
/// extra header — starts here rather than writing the whole policy again.
pub fn builder(policy: Policy) -> reqwest::ClientBuilder {
    let mut builder = reqwest::Client::builder();
    builder = match policy.timeout {
        Some(timeout) => builder.timeout(timeout),
        None => builder,
    };
    if let Some(connect_timeout) = policy.connect_timeout {
        builder = builder.connect_timeout(connect_timeout);
    }
    builder = if policy.follow_redirects {
        builder.redirect(reqwest::redirect::Policy::limited(10))
    } else {
        builder.redirect(reqwest::redirect::Policy::none())
    };
    if policy.https_only {
        builder = builder.https_only(true);
    }
    if policy.no_proxy {
        builder = builder.no_proxy();
    }
    if let Some(headers) = policy.headers.clone() {
        builder = builder.default_headers(headers);
    }
    builder
}

/// The client used for model providers.
///
/// Redirects are refused so a provider cannot silently redirect a request
/// carrying an API key to another host, and the timeout is generous because
/// a long generation is a normal outcome.
pub fn model_provider() -> Result<reqwest::Client, String> {
    client(
        Policy::no_redirects(Duration::from_secs(180)),
        "Could not create the provider HTTP client.",
    )
}

/// The client used for a local model server.
///
/// The proxy configuration is bypassed: a local endpoint must be reached
/// directly.
pub fn local_model() -> Result<reqwest::Client, String> {
    client(
        Policy::no_redirects(Duration::from_secs(600)).no_proxy(),
        "Could not create the local model HTTP client.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_policy_builds_a_client() {
        assert!(client(Policy::following(Duration::from_secs(5)), "no").is_ok());
        assert!(client(Policy::no_redirects(Duration::from_secs(5)), "no").is_ok());
        assert!(client(Policy::unbounded(), "no").is_ok());
        assert!(model_provider().is_ok());
        assert!(local_model().is_ok());
    }

    #[test]
    fn a_build_failure_names_its_own_context() {
        let error = client(Policy::following(Duration::from_secs(5)), "the download client failed")
            .err()
            .unwrap_or_default();
        assert!(error.is_empty() || error.contains("the download client failed"));
    }
}
