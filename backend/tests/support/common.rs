#![allow(
    dead_code,
    reason = "each test crate uses its own subset of these helpers"
)]

#[must_use]
pub fn config(url: &str, extra: &str) -> domain::config::EndpointConfig {
    toml::from_str(&format!("url='{url}'\n{extra}")).unwrap()
}

/// POST requests only: detection also sends GET /props.
pub async fn posts(server: &wiremock::MockServer) -> Vec<wiremock::Request> {
    let requests = server.received_requests().await.unwrap();
    requests
        .into_iter()
        .filter(|r| r.method == "POST")
        .collect()
}
