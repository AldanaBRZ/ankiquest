//! Private site access is deliberately separate from each player's write permission.
//! Browser sessions unlock the shared pages; personal endpoints still require the
//! owning player's bearer token in main::authorized.
use crate::{App, Config, Error, now_ms};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};

const COOKIE: &str = "ankiquest_session";
const SESSION_SECONDS: i64 = 7 * 24 * 60 * 60;
const MAX_SESSIONS: usize = 4096;
const FAILURE_WINDOW_MS: i64 = 60_000;
const MAX_FAILURES: u32 = 30;
const MAX_FAILURE_CLIENTS: usize = 4096;

#[derive(Default)]
struct Sessions {
    expires: HashMap<String, i64>,
    failures: HashMap<IpAddr, Attempts>,
}

struct Attempts {
    started: i64,
    failures: u32,
}

#[derive(Default)]
pub(crate) struct Access {
    password: Option<String>,
    sessions: Mutex<Sessions>,
}

impl Access {
    pub(crate) fn from_config(config: &Config) -> Result<Self, Error> {
        let password = config
            .site_password_file
            .as_ref()
            .map(|path| {
                let password = std::fs::read_to_string(path)
                    .map_err(|e| format!("cannot read site_password_file: {e}"))?;
                let password = password.trim().to_string();
                if password.is_empty() {
                    return Err(Error::from("site_password_file must not be empty"));
                }
                Ok(password)
            })
            .transpose()?;
        if config.private_site
            && password.is_none()
            && !config.users.values().any(|user| {
                user.token
                    .as_ref()
                    .is_some_and(|token| !token.trim().is_empty())
            })
        {
            return Err(
                "private_site needs a nonempty site_password_file or at least one player token"
                    .into(),
            );
        }
        Ok(Self {
            password,
            sessions: Mutex::new(Sessions::default()),
        })
    }

    fn authenticated(&self, headers: &HeaderMap, now: i64) -> bool {
        session_cookie(headers).is_some_and(|cookie| {
            let mut sessions = self.sessions.lock().unwrap();
            match sessions.expires.get(cookie) {
                Some(expires) if *expires > now => true,
                _ => {
                    sessions.expires.remove(cookie);
                    false
                }
            }
        })
    }

    fn throttled(&self, client: IpAddr, now: i64) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.failures.retain(|_, attempts| {
            now >= attempts.started && now - attempts.started < FAILURE_WINDOW_MS
        });
        sessions
            .failures
            .get(&client)
            .is_some_and(|attempts| attempts.failures >= MAX_FAILURES)
    }

    fn failure(&self, client: IpAddr, now: i64) {
        let mut sessions = self.sessions.lock().unwrap();
        if !sessions.failures.contains_key(&client)
            && sessions.failures.len() >= MAX_FAILURE_CLIENTS
            && let Some(oldest) = sessions
                .failures
                .iter()
                .min_by_key(|(_, attempts)| attempts.started)
                .map(|(ip, _)| *ip)
        {
            sessions.failures.remove(&oldest);
        }
        let attempts = sessions.failures.entry(client).or_insert(Attempts {
            started: now,
            failures: 0,
        });
        attempts.failures = attempts.failures.saturating_add(1);
    }

    fn create_session(&self, headers: &HeaderMap, now: i64) -> Result<String, Error> {
        let mut random = [0u8; 32];
        getrandom::getrandom(&mut random).map_err(|_| "session randomness unavailable")?;
        let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let mut sessions = self.sessions.lock().unwrap();
        sessions.expires.retain(|_, expires| *expires > now);
        // Rotate an existing browser session when signing in again.
        if let Some(old) = session_cookie(headers) {
            sessions.expires.remove(old);
        }
        if sessions.expires.len() >= MAX_SESSIONS
            && let Some(oldest) = sessions
                .expires
                .iter()
                .min_by_key(|(_, expires)| *expires)
                .map(|(key, _)| key.clone())
        {
            sessions.expires.remove(&oldest);
        }
        sessions
            .expires
            .insert(token.clone(), now + SESSION_SECONDS * 1000);
        Ok(token)
    }
}

fn constant_eq(expected: &str, given: &str) -> bool {
    !expected.is_empty()
        && expected.len() == given.len()
        && expected
            .bytes()
            .zip(given.bytes())
            .fold(0, |acc, (a, b)| acc | (a ^ b))
            == 0
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

fn member_token(app: &App, given: &str) -> bool {
    app.config
        .users
        .values()
        .filter_map(|user| user.token.as_deref())
        .fold(false, |matched, expected| {
            constant_eq(expected, given) | matched
        })
}

fn authenticated(app: &App, headers: &HeaderMap) -> bool {
    bearer(headers).is_some_and(|given| member_token(app, given))
        || app.access.authenticated(headers, now_ms())
}

// A duplicate cookie is ambiguous (for example, a sibling subdomain's cookie),
// so never choose one based on header or path ordering.
fn session_cookie(headers: &HeaderMap) -> Option<&str> {
    let mut found = None;
    for header in headers.get_all(header::COOKIE) {
        for cookie in header.to_str().ok()?.split(';') {
            if let Some((name, value)) = cookie.trim().split_once('=')
                && name == COOKIE
            {
                if found.is_some()
                    || value.len() != 64
                    || !value.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return None;
                }
                found = Some(value);
            }
        }
    }
    found
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({"error": message}))).into_response()
}

fn public_path(path: &str) -> bool {
    matches!(
        path,
        "/login"
            | "/site.css"
            | "/site.js"
            | "/icon.svg"
            | "/manifest.webmanifest"
            | "/auth/status"
            | "/auth/session"
            | "/auth/logout"
    )
}

fn encoded_next(path: &str) -> String {
    path.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

pub(crate) async fn gate(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let mut response = if app.config.private_site
        && !public_path(path)
        && !authenticated(&app, request.headers())
    {
        if path.starts_with("/api/") || !matches!(*request.method(), Method::GET | Method::HEAD) {
            error(
                StatusCode::UNAUTHORIZED,
                "Sign in to access this private community.",
            )
        } else {
            let destination = request
                .uri()
                .path_and_query()
                .map_or("/", |path| path.as_str());
            Redirect::to(&format!("/login?next={}", encoded_next(destination))).into_response()
        }
    } else {
        next.run(request).await
    };
    // Data and authentication responses must never survive logout in a shared cache.
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn origin(value: &str) -> Option<String> {
    let uri: Uri = value.parse().ok()?;
    let scheme = uri.scheme_str()?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    let authority = uri.authority()?.as_str();
    if authority.contains('@') {
        return None;
    }
    Some(format!("{scheme}://{authority}"))
}

fn same_origin(app: &App, headers: &HeaderMap) -> bool {
    let Some(given) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Ok(given) = given.to_str() else {
        return false;
    };
    let expected = app
        .config
        .public_url
        .as_deref()
        .and_then(origin)
        .or_else(|| {
            let host = headers.get(header::HOST)?.to_str().ok()?;
            origin(&format!("http://{host}"))
        });
    expected.as_deref() == Some(given)
}

fn csrf(headers: &HeaderMap) -> bool {
    headers
        .get("x-ankiquest-csrf")
        .is_some_and(|value| value == "1")
}

fn cookie(app: &App, value: &str, max_age: i64) -> HeaderValue {
    let secure = app
        .config
        .public_url
        .as_deref()
        .is_some_and(|url| url.starts_with("https://"));
    format!(
        "{COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{}",
        if secure { "; Secure" } else { "" }
    )
    .parse()
    .unwrap()
}

pub(crate) async fn status(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"private_site": app.config.private_site, "authenticated": authenticated(&app, &headers)}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Password {
    password: String,
}

// A reverse proxy may identify the original client only when explicitly trusted
// and reached over a loopback connection. Ignore spoofed or ambiguous headers.
fn client_ip(config: &Config, headers: &HeaderMap, peer: SocketAddr) -> IpAddr {
    if config.site_trust_proxy && peer.ip().is_loopback() {
        let mut values = headers.get_all("x-ankiquest-client-ip").iter();
        if let Some(value) = values.next()
            && values.next().is_none()
            && let Some(ip) = value.to_str().ok().and_then(|value| value.parse().ok())
        {
            return ip;
        }
    }
    peer.ip()
}
pub(crate) async fn session(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !same_origin(&app, &headers) {
        return error(StatusCode::FORBIDDEN, "Sign in from this site.");
    }
    let valid_token = bearer(&headers).is_some_and(|given| member_token(&app, given));
    if bearer(&headers).is_some() && !valid_token {
        return error(StatusCode::UNAUTHORIZED, "Token not recognized.");
    }
    // A password attacker cannot lock configured apps out of session bootstrap.
    if !valid_token {
        if !csrf(&headers) {
            return error(StatusCode::FORBIDDEN, "Missing sign-in request header.");
        }
        let now = now_ms();
        let client = client_ip(&app.config, &headers, peer);
        if app.access.throttled(client, now) {
            let mut response = error(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many attempts. Try again in one minute.",
            );
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
            return response;
        }
        let credential = serde_json::from_slice::<Password>(&body).ok();
        let valid = credential.as_ref().is_some_and(|credential| {
            credential.password.len() <= 1024
                && (member_token(&app, &credential.password)
                    || app
                        .access
                        .password
                        .as_deref()
                        .is_some_and(|expected| constant_eq(expected, &credential.password)))
        });
        if !valid {
            app.access.failure(client, now);
            return error(
                StatusCode::UNAUTHORIZED,
                "Password or token not recognized.",
            );
        }
    }
    match app.access.create_session(&headers, now_ms()) {
        Ok(token) => (
            StatusCode::NO_CONTENT,
            [(header::SET_COOKIE, cookie(&app, &token, SESSION_SECONDS))],
        )
            .into_response(),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to create a session.",
        ),
    }
}

pub(crate) async fn logout(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if !csrf(&headers) || !same_origin(&app, &headers) {
        return error(StatusCode::FORBIDDEN, "Sign out from this site.");
    }
    if let Some(session) = session_cookie(&headers) {
        app.access.sessions.lock().unwrap().expires.remove(session);
    }
    (
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, cookie(&app, "", 0))],
    )
        .into_response()
}

pub(crate) async fn login() -> Html<&'static str> {
    Html(include_str!("../static/login.html"))
}
pub(crate) async fn site_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../static/site.css"),
    )
}
pub(crate) async fn site_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../static/site.js"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Week;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use std::path::PathBuf;
    use std::sync::RwLock;
    use tower::ServiceExt;

    fn fixture(private_site: bool) -> (Arc<App>, PathBuf) {
        let (store, path) = crate::decks::tests::temporary_store();
        let config = serde_json::from_value(serde_json::json!({
            "private_site": private_site,
            "public_url": "https://anki.example.test",
            "users": {"alice": {"token": "alice-token"}, "bob": {"token": "bob-token"}}
        }))
        .unwrap();
        (
            Arc::new(App {
                config,
                access: Access {
                    password: Some("shared-master".into()),
                    ..Access::default()
                },
                week: Week::default(),
                store: Mutex::new(store),
                players: RwLock::new(HashMap::new()),
            }),
            path,
        )
    }

    async fn request(
        app: &Arc<App>,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Response {
        request_from(
            app,
            method,
            path,
            headers,
            body,
            "192.0.2.1:40000".parse().unwrap(),
        )
        .await
    }

    async fn request_from(
        app: &Arc<App>,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
        peer: SocketAddr,
    ) -> Response {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .extension(ConnectInfo(peer))
            .header("host", "anki.example.test");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        crate::router(app.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap()
    }

    async fn sign_in(app: &Arc<App>, password: &str) -> Response {
        request(
            app,
            "POST",
            "/auth/session",
            &[
                ("x-ankiquest-csrf", "1"),
                ("origin", "https://anki.example.test"),
                ("content-type", "application/json"),
            ],
            &serde_json::json!({"password": password}).to_string(),
        )
        .await
    }

    fn session_header(response: &Response) -> String {
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    fn cleanup(app: Arc<App>, path: PathBuf) {
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn private_router_closes_every_data_route_but_keeps_login_assets_public() {
        let (app, path) = fixture(true);
        for endpoint in [
            "/api/leaderboard",
            "/api/records",
            "/api/winners",
            "/api/community",
            "/api/week",
            "/api/profile/alice",
            "/api/notifications/alice",
            "/api/decks/alice",
            "/api/community/reminders/alice",
            "/api/community/challenges/alice",
            "/api/streak-freezes/alice",
        ] {
            let response = request(&app, "GET", endpoint, &[], "").await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{endpoint}");
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            let body = to_bytes(response.into_body(), 4096).await.unwrap();
            assert!(
                serde_json::from_slice::<serde_json::Value>(&body).unwrap()["error"].is_string()
            );
        }
        for endpoint in [
            "/api/reviews/alice",
            "/api/preview/alice",
            "/api/reply/alice",
        ] {
            assert_eq!(
                request(&app, "POST", endpoint, &[], "{}").await.status(),
                StatusCode::UNAUTHORIZED
            );
        }
        for endpoint in ["/", "/records", "/community", "/week", "/month"] {
            let response = request(&app, "GET", endpoint, &[], "").await;
            assert_eq!(response.status(), StatusCode::SEE_OTHER, "{endpoint}");
            assert!(
                response.headers()[header::LOCATION]
                    .to_str()
                    .unwrap()
                    .starts_with("/login?next=")
            );
        }
        for endpoint in [
            "/login",
            "/site.css",
            "/site.js",
            "/icon.svg",
            "/manifest.webmanifest",
            "/auth/status",
        ] {
            assert_eq!(
                request(&app, "GET", endpoint, &[], "").await.status(),
                StatusCode::OK,
                "{endpoint}"
            );
        }
        cleanup(app, path);
    }

    #[tokio::test]
    async fn master_and_token_sessions_read_shared_data_but_never_unlock_personal_endpoints() {
        let (app, path) = fixture(true);
        for password in ["shared-master", "alice-token"] {
            let response = sign_in(&app, password).await;
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            let raw = response.headers()[header::SET_COOKIE].to_str().unwrap();
            for attribute in [
                "HttpOnly",
                "SameSite=Lax",
                "Secure",
                "Path=/",
                "Max-Age=604800",
            ] {
                assert!(raw.contains(attribute));
            }
            let cookie = session_header(&response);
            assert_eq!(
                request(&app, "GET", "/api/leaderboard", &[("cookie", &cookie)], "")
                    .await
                    .status(),
                StatusCode::OK
            );
            for endpoint in [
                "/api/notifications/alice",
                "/api/community/reminders/alice",
                "/api/decks/alice",
            ] {
                assert_eq!(
                    request(&app, "GET", endpoint, &[("cookie", &cookie)], "")
                        .await
                        .status(),
                    StatusCode::UNAUTHORIZED
                );
            }
            assert_eq!(
                request(
                    &app,
                    "POST",
                    "/api/streak-freezes/alice",
                    &[("cookie", &cookie), ("content-type", "application/json")],
                    r#"{"enabled":true}"#
                )
                .await
                .status(),
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(
            request(
                &app,
                "GET",
                "/api/leaderboard",
                &[("authorization", "Bearer shared-master")],
                ""
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(
                &app,
                "GET",
                "/api/decks/alice",
                &[("authorization", "Bearer bob-token")],
                ""
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(
                &app,
                "GET",
                "/api/decks/alice",
                &[("authorization", "Bearer alice-token")],
                ""
            )
            .await
            .status(),
            StatusCode::OK
        );
        cleanup(app, path);
    }

    #[tokio::test]
    async fn sessions_reject_cross_origin_requests_expire_and_are_revoked_at_logout() {
        let (app, path) = fixture(true);
        assert_eq!(
            request(
                &app,
                "POST",
                "/auth/session",
                &[
                    ("x-ankiquest-csrf", "1"),
                    ("origin", "https://evil.example")
                ],
                r#"{"password":"shared-master"}"#
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            request(
                &app,
                "POST",
                "/auth/session",
                &[],
                r#"{"password":"shared-master"}"#
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let response = sign_in(&app, "shared-master").await;
        let cookie = session_header(&response);
        let duplicate = format!("{cookie}; {cookie}");
        assert_eq!(
            request(
                &app,
                "GET",
                "/api/leaderboard",
                &[("cookie", &duplicate)],
                ""
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(&app, "POST", "/auth/logout", &[("cookie", &cookie)], "")
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        let logout = request(
            &app,
            "POST",
            "/auth/logout",
            &[
                ("cookie", &cookie),
                ("x-ankiquest-csrf", "1"),
                ("origin", "https://anki.example.test"),
            ],
            "",
        )
        .await;
        assert_eq!(logout.status(), StatusCode::NO_CONTENT);
        assert!(
            logout.headers()[header::SET_COOKIE]
                .to_str()
                .unwrap()
                .contains("Max-Age=0")
        );
        assert_eq!(
            request(&app, "GET", "/api/leaderboard", &[("cookie", &cookie)], "")
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let response = sign_in(&app, "alice-token").await;
        let cookie = session_header(&response);
        app.access
            .sessions
            .lock()
            .unwrap()
            .expires
            .values_mut()
            .for_each(|expiry| *expiry = now_ms() - 1);
        assert_eq!(
            request(&app, "GET", "/api/leaderboard", &[("cookie", &cookie)], "")
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        cleanup(app, path);
    }

    #[tokio::test]
    async fn password_attempts_are_limited_while_native_bearer_bootstrap_keeps_working() {
        let (app, path) = fixture(true);
        for _ in 0..MAX_FAILURES {
            assert_eq!(
                sign_in(&app, "wrong").await.status(),
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(
            sign_in(&app, "wrong").await.status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        let response = request(
            &app,
            "POST",
            "/auth/session",
            &[("authorization", "Bearer alice-token")],
            "",
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let cookie = session_header(&response);
        assert_eq!(
            request(&app, "GET", "/api/leaderboard", &[("cookie", &cookie)], "")
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            request(
                &app,
                "GET",
                "/api/leaderboard",
                &[("authorization", "Bearer bob-token")],
                ""
            )
            .await
            .status(),
            StatusCode::OK
        );
        app.access
            .sessions
            .lock()
            .unwrap()
            .failures
            .values_mut()
            .for_each(|attempts| attempts.started -= FAILURE_WINDOW_MS);
        assert_eq!(
            sign_in(&app, "shared-master").await.status(),
            StatusCode::NO_CONTENT
        );
        cleanup(app, path);
    }

    #[tokio::test]
    async fn password_throttling_is_per_client_and_ignores_untrusted_forwarded_headers() {
        let (app, path) = fixture(true);
        for _ in 0..MAX_FAILURES {
            assert_eq!(
                sign_in(&app, "wrong").await.status(),
                StatusCode::UNAUTHORIZED
            );
        }
        let headers = [
            ("x-ankiquest-csrf", "1"),
            ("x-ankiquest-client-ip", "198.51.100.9"),
        ];
        let body = r#"{"password":"shared-master"}"#;
        // An attacker cannot rotate a request header to escape their own limit.
        assert_eq!(
            request(&app, "POST", "/auth/session", &headers, body)
                .await
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        // A separate member still signs in while the first client is blocked.
        assert_eq!(
            request_from(
                &app,
                "POST",
                "/auth/session",
                &headers,
                body,
                "192.0.2.2:40000".parse().unwrap()
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        cleanup(app, path);
    }

    #[tokio::test]
    async fn trusted_loopback_proxy_keeps_browser_failure_buckets_separate() {
        let (mut app, path) = fixture(true);
        Arc::get_mut(&mut app).unwrap().config.site_trust_proxy = true;
        let peer = "127.0.0.1:40000".parse().unwrap();
        let attacker = [
            ("x-ankiquest-csrf", "1"),
            ("x-ankiquest-client-ip", "198.51.100.1"),
        ];
        for _ in 0..MAX_FAILURES {
            assert_eq!(
                request_from(
                    &app,
                    "POST",
                    "/auth/session",
                    &attacker,
                    r#"{"password":"wrong"}"#,
                    peer
                )
                .await
                .status(),
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(
            request_from(
                &app,
                "POST",
                "/auth/session",
                &attacker,
                r#"{"password":"wrong"}"#,
                peer
            )
            .await
            .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        let member = [
            ("x-ankiquest-csrf", "1"),
            ("x-ankiquest-client-ip", "198.51.100.2"),
        ];
        assert_eq!(
            request_from(
                &app,
                "POST",
                "/auth/session",
                &member,
                r#"{"password":"shared-master"}"#,
                peer
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        cleanup(app, path);
    }

    #[test]
    fn proxy_identity_requires_opt_in_loopback_and_one_valid_ip() {
        let (app, path) = fixture(true);
        let mut config = app.config.clone();
        let mut headers = HeaderMap::new();
        headers.insert("x-ankiquest-client-ip", "198.51.100.1".parse().unwrap());
        let local: SocketAddr = "127.0.0.1:40000".parse().unwrap();
        let remote: SocketAddr = "192.0.2.1:40000".parse().unwrap();
        assert_eq!(client_ip(&config, &headers, local), local.ip());
        config.site_trust_proxy = true;
        assert_eq!(
            client_ip(&config, &headers, local),
            "198.51.100.1".parse::<IpAddr>().unwrap()
        );
        assert_eq!(client_ip(&config, &headers, remote), remote.ip());
        headers.append("x-ankiquest-client-ip", "198.51.100.2".parse().unwrap());
        assert_eq!(client_ip(&config, &headers, local), local.ip());
        headers.insert(
            "x-ankiquest-client-ip",
            "198.51.100.1, 198.51.100.2".parse().unwrap(),
        );
        assert_eq!(client_ip(&config, &headers, local), local.ip());
        cleanup(app, path);
    }

    #[test]
    fn password_failure_buckets_expire_and_have_a_fixed_memory_bound() {
        let access = Access::default();
        for client in 0..=MAX_FAILURE_CLIENTS {
            access.failure(IpAddr::V4(std::net::Ipv4Addr::from(client as u32)), 1000);
        }
        assert_eq!(
            access.sessions.lock().unwrap().failures.len(),
            MAX_FAILURE_CLIENTS
        );
        assert!(!access.throttled("192.0.2.1".parse().unwrap(), 1000 + FAILURE_WINDOW_MS));
        assert!(access.sessions.lock().unwrap().failures.is_empty());
    }
    #[tokio::test]
    async fn public_mode_remains_compatible_and_can_bootstrap_a_future_private_session() {
        let (app, path) = fixture(false);
        assert_eq!(
            request(&app, "GET", "/api/leaderboard", &[], "")
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            request(
                &app,
                "POST",
                "/auth/session",
                &[("authorization", "Bearer alice-token")],
                ""
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        cleanup(app, path);
    }

    #[test]
    fn private_configuration_requires_real_credentials_and_rejects_empty_password_files() {
        let empty: Config =
            serde_json::from_value(serde_json::json!({"private_site": true})).unwrap();
        assert!(Access::from_config(&empty).is_err());
        let empty_token: Config = serde_json::from_value(
            serde_json::json!({"private_site": true, "users": {"alice": {"token": ""}}}),
        )
        .unwrap();
        assert!(Access::from_config(&empty_token).is_err());
        let token: Config = serde_json::from_value(
            serde_json::json!({"private_site": true, "users": {"alice": {"token": "secret"}}}),
        )
        .unwrap();
        assert!(Access::from_config(&token).is_ok());
        let (store, path) = crate::decks::tests::temporary_store();
        let password_file = path.join("password");
        std::fs::write(&password_file, "  \n").unwrap();
        let mut config = token;
        config.site_password_file = Some(password_file.clone());
        assert!(Access::from_config(&config).is_err());
        std::fs::write(&password_file, "shared-master\n").unwrap();
        assert!(Access::from_config(&config).is_ok());
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }
}
