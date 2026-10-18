//! `net.fetch` for the runner: answers from a file of canned responses and
//! applies the checks slim's own host makes before it dials anything.
//!
//! The hosts a module may reach are the hosts of the URLs in the file, the way
//! a manifest's `network.hosts` is what an admin approves. Nothing here opens
//! a socket. The constants mirror `module_runtime/net.rs` in slim-m, and a test
//! pins them so a drift is a failing test rather than a surprise on install.

use std::collections::BTreeMap;

use serde_json::{json, Value};

/// Most requests one run may make.
pub const MAX_FETCHES_PER_RUN: u32 = 4;
/// Largest request body a module may send.
pub const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024;
/// Largest response body handed back; more is cut and flagged `truncated`.
pub const MAX_RESPONSE_BODY_BYTES: usize = 256 * 1024;
const MAX_REQUEST_HEADERS: usize = 16;
const RESERVED_HEADERS: [&str; 5] = ["host", "content-length", "connection", "transfer-encoding", "upgrade"];
const METHODS: [&str; 6] = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE"];

/// The canned responses, keyed by full URL.
#[derive(Default)]
pub struct Canned {
    responses: BTreeMap<String, Value>,
    hosts: Vec<String>,
    fetches: u32,
}

impl Canned {
    pub fn parse(text: &str) -> Result<Self, String> {
        let Value::Object(map) = serde_json::from_str::<Value>(text).map_err(|e| format!("fetch file is not JSON: {e}"))? else {
            return Err("fetch file must be a JSON object keyed by URL".to_owned());
        };
        let mut canned = Canned::default();
        for (url, response) in map {
            let (_, entry, _) = split_url(&url).ok_or_else(|| format!("fetch file key is not an http(s) URL: {url}"))?;
            if !canned.hosts.contains(&entry) {
                canned.hosts.push(entry);
            }
            canned.responses.insert(url, response);
        }
        Ok(canned)
    }

    /// Answers one `net.fetch` request with slim's wire shape.
    pub fn handle(&mut self, request: &str) -> String {
        if self.fetches >= MAX_FETCHES_PER_RUN {
            return refusal("net.fetch budget for this run is exhausted");
        }
        self.fetches += 1;
        match self.answer(request) {
            Ok(response) => response.to_string(),
            Err(reason) => refusal(&reason),
        }
    }

    fn answer(&self, request: &str) -> Result<Value, String> {
        let raw: Value = serde_json::from_str(request).map_err(|_| "malformed net.fetch request".to_owned())?;
        let url = raw["url"].as_str().ok_or("net.fetch needs a url")?;
        let (scheme_ok, entry, key) = split_url(url).ok_or("net.fetch url is not a valid URL")?;
        if !scheme_ok {
            return Err("net.fetch allows only http and https".into());
        }
        if url.split("//").nth(1).is_some_and(|rest| rest.split('/').next().is_some_and(|a| a.contains('@'))) {
            return Err("net.fetch url may not carry credentials".into());
        }
        if !self.hosts.contains(&entry) {
            let host = entry.split(':').next().unwrap_or(&entry);
            return Err(format!("host not approved for this module: {host}"));
        }
        let method = raw["method"].as_str().unwrap_or("GET").to_ascii_uppercase();
        if !METHODS.contains(&method.as_str()) {
            return Err("net.fetch method not allowed".into());
        }
        check_headers(&raw["headers"])?;
        if raw["body"].as_str().is_some_and(|b| b.len() > MAX_REQUEST_BODY_BYTES) {
            return Err("net.fetch request body is too large".into());
        }
        let canned = self.responses.get(url).or_else(|| self.responses.get(&key));
        Ok(response(canned))
    }
}

fn check_headers(headers: &Value) -> Result<(), String> {
    let Some(map) = headers.as_object() else { return Ok(()) };
    if map.len() > MAX_REQUEST_HEADERS {
        return Err("net.fetch sets too many headers".into());
    }
    for name in map.keys() {
        let lower = name.to_ascii_lowercase();
        if RESERVED_HEADERS.contains(&lower.as_str()) {
            return Err(format!("net.fetch may not set the {lower} header"));
        }
    }
    Ok(())
}

/// A missing entry answers 404, as an upstream with nothing there would.
fn response(canned: Option<&Value>) -> Value {
    let status = canned.and_then(|c| c["status"].as_u64()).unwrap_or(if canned.is_some() { 200 } else { 404 });
    let body = canned.and_then(|c| c["body"].as_str()).unwrap_or("");
    let cut = floor_char_boundary(body, MAX_RESPONSE_BODY_BYTES);
    json!({
        "ok": true,
        "status": status,
        "content_type": canned.and_then(|c| c["content_type"].as_str()),
        "location": canned.and_then(|c| c["location"].as_str()),
        "body": &body[..cut],
        "truncated": cut < body.len(),
    })
}

fn floor_char_boundary(text: &str, at: usize) -> usize {
    (0..=at.min(text.len())).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0)
}

fn refusal(message: &str) -> String {
    json!({ "ok": false, "error": message }).to_string()
}

/// `(scheme is http or https, "host" or "host:port", url without a fragment)`.
/// A default port is dropped, so `https://h:443/` and `https://h/` are one host.
fn split_url(url: &str) -> Option<(bool, String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?;
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => (h, Some(p)),
        _ => (authority, None),
    };
    if host.is_empty() {
        return None;
    }
    let default = if scheme.eq_ignore_ascii_case("https") { "443" } else { "80" };
    let entry = match port {
        Some(p) if p != default => format!("{}:{p}", host.to_ascii_lowercase()),
        _ => host.to_ascii_lowercase(),
    };
    let ok = scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https");
    Some((ok, entry, url.split('#').next()?.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canned() -> Canned {
        Canned::parse(r#"{"https://api.example.com/v1/up": {"status": 200, "content_type": "application/json", "body": "{\"up\":true}"}}"#).unwrap()
    }

    #[test]
    fn a_listed_url_returns_its_canned_response() {
        let out = canned().handle(r#"{"url":"https://api.example.com/v1/up"}"#);
        assert!(out.contains(r#""status":200"#) && out.contains(r#"\"up\":true"#), "{out}");
    }

    #[test]
    fn an_unlisted_host_is_refused_like_slim_refuses_it() {
        let out = canned().handle(r#"{"url":"https://evil.example.org/"}"#);
        assert!(out.contains("host not approved for this module: evil.example.org"), "{out}");
        let out = canned().handle(r#"{"url":"https://api.example.com:8443/v1/up"}"#);
        assert!(out.contains("host not approved"), "{out}");
    }

    #[test]
    fn a_listed_host_with_nothing_at_that_path_answers_404() {
        let out = canned().handle(r#"{"url":"https://api.example.com/other"}"#);
        assert!(out.contains(r#""status":404"#), "{out}");
    }

    #[test]
    fn the_hosts_requests_and_budget_are_policed() {
        let mut c = canned();
        for bad in [
            r#"{"url":"ftp://api.example.com/"}"#,
            r#"{"url":"https://u:p@api.example.com/"}"#,
            r#"{"url":"https://api.example.com/","method":"TRACE"}"#,
            r#"{"url":"https://api.example.com/","headers":{"Host":"x"}}"#,
        ] {
            assert!(c.handle(bad).contains(r#""ok":false"#), "{bad}");
        }
        let mut c = canned();
        for _ in 0..MAX_FETCHES_PER_RUN {
            c.handle(r#"{"url":"https://api.example.com/v1/up"}"#);
        }
        assert!(c.handle(r#"{"url":"https://api.example.com/v1/up"}"#).contains("budget"));
    }

    #[test]
    fn a_large_body_is_cut_and_flagged() {
        let big = "x".repeat(MAX_RESPONSE_BODY_BYTES + 5);
        let file = format!(r#"{{"https://api.example.com/big": {{"body": "{big}"}}}}"#);
        let out = Canned::parse(&file).unwrap().handle(r#"{"url":"https://api.example.com/big"}"#);
        assert!(out.contains(r#""truncated":true"#), "{}", &out[..80]);
    }

    /// The numbers slim-m's `module_runtime/net.rs` enforces; change both together.
    #[test]
    fn the_limits_match_slims() {
        assert_eq!(MAX_FETCHES_PER_RUN, 4);
        assert_eq!(MAX_REQUEST_BODY_BYTES, 64 * 1024);
        assert_eq!(MAX_RESPONSE_BODY_BYTES, 256 * 1024);
        assert_eq!(MAX_REQUEST_HEADERS, 16);
    }
}
