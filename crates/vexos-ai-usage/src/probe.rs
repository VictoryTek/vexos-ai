//! The one request: `GET https://api.anthropic.com/api/oauth/usage` with the
//! access token and a beta header.
//!
//! The host, path and header are constants. There is no setting, environment
//! variable or argument that moves the request, so the token can only ever go
//! where it is meant to. The request is https only, follows no redirect, is
//! cut off after 10 s, and reads at most [`MAX_BODY`] bytes of the answer.

use std::time::Duration;

use crate::secret::Secret;

pub const ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";
const BETA: &str = "oauth-2025-04-20";
pub const MAX_BODY: u64 = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);

/// Why a request gave no answer. None of these carries a token or any text of
/// the other side's: the panel shows its own fixed sentences.
#[derive(Debug, PartialEq, Eq)]
pub enum FetchError {
    /// The server answered with an error status. `retry_after_secs` is the
    /// `Retry-After` header when it is a number of seconds.
    Status {
        code: u16,
        retry_after_secs: Option<u64>,
    },
    /// Nothing answered: no route, no DNS, a refused or dropped connection, a
    /// timeout. The only failure worth retrying soon.
    Transport,
    /// The answer was bigger than [`MAX_BODY`].
    TooLarge,
    /// The server tried to send the request somewhere else. Never followed.
    Redirect,
    /// Anything else: an unreadable answer, a bad certificate.
    Other,
}

/// Fetches the usage document. A trait so the rules above `Fetch` are tested
/// without a network.
pub trait Fetch {
    fn fetch(&self, token: &Secret) -> Result<String, FetchError>;
}

/// The real thing. `https_only` is off only in this crate's own tests, which
/// talk to a socket on this machine.
pub struct Anthropic {
    url: String,
    https_only: bool,
}

impl Anthropic {
    pub fn new() -> Anthropic {
        Anthropic {
            url: ENDPOINT.to_owned(),
            https_only: true,
        }
    }

    #[cfg(test)]
    fn for_test(url: String) -> Anthropic {
        Anthropic {
            url,
            https_only: false,
        }
    }
}

impl Default for Anthropic {
    fn default() -> Self {
        Anthropic::new()
    }
}

fn classify(error: &ureq::Error) -> FetchError {
    match error {
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => FetchError::Redirect,
        ureq::Error::BodyExceedsLimit(_) => FetchError::TooLarge,
        ureq::Error::Timeout(_)
        | ureq::Error::Io(_)
        | ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed => FetchError::Transport,
        _ => FetchError::Other,
    }
}

impl Fetch for Anthropic {
    fn fetch(&self, token: &Secret) -> Result<String, FetchError> {
        let config = ureq::Agent::config_builder()
            .max_redirects(0)
            // The token goes straight to Anthropic, never through a proxy that the
            // environment names.
            .proxy(None)
            .https_only(self.https_only)
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .build();
        let agent: ureq::Agent = config.into();
        // The one place the token leaves this crate.
        let mut response = agent
            .get(&self.url)
            .header("Authorization", format!("Bearer {}", token.expose()))
            .header("anthropic-beta", BETA)
            .header("Accept", "application/json")
            .call()
            .map_err(|e| classify(&e))?;
        let code = response.status().as_u16();
        if (300..400).contains(&code) {
            return Err(FetchError::Redirect);
        }
        if !(200..300).contains(&code) {
            let retry_after_secs = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok());
            return Err(FetchError::Status {
                code,
                retry_after_secs,
            });
        }
        response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_string()
            .map_err(|e| classify(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    /// Serves one canned reply on a local socket and hands back the request it
    /// got, so the headers can be checked.
    fn serve(reply: Vec<u8>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api/oauth/usage", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
                let _ = stream.write_all(&reply);
            }
        });
        (url, rx)
    }

    fn response(status: &str, headers: &str, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    fn token() -> Secret {
        Secret::new("tok-123".to_owned())
    }

    #[test]
    fn sends_the_token_and_the_beta_header_and_returns_the_body() {
        let (url, seen) = serve(response("200 OK", "", r#"{"five_hour":{}}"#));
        let body = Anthropic::for_test(url).fetch(&token()).unwrap();
        assert_eq!(body, r#"{"five_hour":{}}"#);
        let request = seen.recv().unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get /api/oauth/usage "), "{request}");
        assert!(
            request.contains("authorization: bearer tok-123"),
            "{request}"
        );
        assert!(
            request.contains("anthropic-beta: oauth-2025-04-20"),
            "{request}"
        );
    }

    #[test]
    fn error_statuses_carry_retry_after_and_nothing_else() {
        let (url, _) = serve(response(
            "429 Too Many Requests",
            "Retry-After: 120\r\n",
            "slow down",
        ));
        assert_eq!(
            Anthropic::for_test(url).fetch(&token()),
            Err(FetchError::Status {
                code: 429,
                retry_after_secs: Some(120)
            })
        );
        let (url, _) = serve(response("401 Unauthorized", "", "no"));
        assert_eq!(
            Anthropic::for_test(url).fetch(&token()),
            Err(FetchError::Status {
                code: 401,
                retry_after_secs: None
            })
        );
        let (url, _) = serve(response(
            "503 Service Unavailable",
            "Retry-After: Wed, 21 Oct 2026 07:28:00 GMT\r\n",
            "",
        ));
        assert_eq!(
            Anthropic::for_test(url).fetch(&token()),
            Err(FetchError::Status {
                code: 503,
                retry_after_secs: None
            })
        );
    }

    #[test]
    fn a_redirect_is_refused_and_never_followed() {
        let (target, seen_by_target) = serve(response("200 OK", "", "{}"));
        let (url, _) = serve(response(
            "302 Found",
            &format!("Location: {target}\r\n"),
            "",
        ));
        assert_eq!(
            Anthropic::for_test(url).fetch(&token()),
            Err(FetchError::Redirect)
        );
        // The redirect target never saw a request, so the token never went there.
        assert!(
            seen_by_target
                .recv_timeout(Duration::from_millis(300))
                .is_err()
        );
    }

    #[test]
    fn an_oversize_answer_is_refused() {
        let big = "x".repeat(usize::try_from(MAX_BODY).unwrap() + 10);
        let (url, _) = serve(response("200 OK", "", &big));
        assert_eq!(
            Anthropic::for_test(url).fetch(&token()),
            Err(FetchError::TooLarge)
        );
    }

    #[test]
    fn nothing_listening_is_a_transport_failure() {
        let url = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}/x", listener.local_addr().unwrap())
        };
        assert_eq!(
            Anthropic::for_test(url).fetch(&token()),
            Err(FetchError::Transport)
        );
    }

    #[test]
    fn plain_http_is_refused_by_the_real_constructor() {
        let (url, seen) = serve(response("200 OK", "", "{}"));
        let real = Anthropic {
            url,
            https_only: true,
        };
        assert!(real.fetch(&token()).is_err());
        assert!(
            seen.recv_timeout(Duration::from_millis(300)).is_err(),
            "the request must not be sent"
        );
    }

    #[test]
    fn the_endpoint_is_a_fixed_https_url_on_anthropics_host() {
        assert_eq!(ENDPOINT, "https://api.anthropic.com/api/oauth/usage");
        assert_eq!(Anthropic::new().url, ENDPOINT);
        assert!(Anthropic::new().https_only);
    }
}
