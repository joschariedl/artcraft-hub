//! Thin wrapper around `ureq`: one place that configures timeouts, status
//! handling and body limits.
//!
//! Every byte that reaches this layer is untrusted. Bodies are capped, non-2xx
//! answers are returned instead of panicking, and callers decide what a 404 or
//! a 429 means.

use std::time::Duration;

use crate::error::{Error, Result};

pub const USER_AGENT: &str = concat!(
    "ArtCraftHub/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/derfleischmann/artcraft-hub)"
);

/// A bounded HTTP answer.
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    /// Seconds from `Retry-After`, when the server sent one.
    pub retry_after: Option<u64>,
}

impl Reply {
    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// An agent with a total timeout. Status codes are *not* turned into errors so
/// the caller can react to 404/429 individually.
pub fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(USER_AGENT)
        .http_status_as_error(false)
        .timeout_global(Some(timeout))
        .build()
        .into()
}

pub fn get(agent: &ureq::Agent, url: &str, headers: &[(&str, &str)], max_body: u64) -> Result<Reply> {
    let mut req = agent.get(url);
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    let response = req.call().map_err(map_error)?;
    read_reply(response, max_body)
}

pub fn post_form(
    agent: &ureq::Agent,
    url: &str,
    headers: &[(&str, &str)],
    form: &[(&str, &str)],
    max_body: u64,
) -> Result<Reply> {
    let mut req = agent.post(url);
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    let response = req.send_form(form.iter().copied()).map_err(map_error)?;
    read_reply(response, max_body)
}

fn read_reply(mut response: ureq::http::Response<ureq::Body>, max_body: u64) -> Result<Reply> {
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok());
    let body = response
        .body_mut()
        .with_config()
        .limit(max_body)
        .read_to_vec()
        .map_err(|e| Error::Http(e.to_string()))?;
    Ok(Reply {
        status,
        body,
        retry_after,
    })
}

fn map_error(error: ureq::Error) -> Error {
    match error {
        ureq::Error::StatusCode(code) => Error::Http(format!("HTTP-Status {code}")),
        other => Error::Http(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_conversion_never_fails() {
        let reply = Reply {
            status: 200,
            body: vec![0xff, b'a'],
            retry_after: None,
        };
        assert!(reply.body_text().contains('a'));
        assert!(reply.is_success());
    }
}
