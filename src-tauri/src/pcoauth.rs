//! Signing in to Planning Center the official way: OAuth 2.0 authorization
//! code with PKCE.
//!
//! What this replaces. ProDeck's original path was a **Personal Access
//! Token** — an Application ID and Secret that a church admin creates on the
//! developer site and pastes in. It works, but it is the wrong shape for an
//! app handed to other churches:
//!
//! - The secret is a long-lived password sitting in `settings.json`, in every
//!   backup bundle, and in whatever the admin pasted it out of.
//! - It carries that person's *entire* Planning Center access. There is no
//!   scope, so a token for reading the Sunday plan can also read Giving.
//! - Nothing in Planning Center shows it is in use, and revoking it means
//!   hunting for the right row on the developer site.
//! - It takes an eight-step detour through a developer site that most worship
//!   pastors have never seen, which is where new churches gave up.
//!
//! OAuth fixes all four: the operator clicks **Connect**, signs in on Planning
//! Center's own page, and approves ProDeck for the `services` and `people`
//! products only. ProDeck stores an access token that dies in two hours and a
//! refresh token it rotates, and the church can see and revoke the connection
//! from their Planning Center account. No password of theirs is ever typed
//! into ProDeck or stored by it.
//!
//! Why PKCE and no client secret. A desktop app cannot keep a secret — anyone
//! who downloads the DMG has the bytes. Planning Center calls that a *public*
//! application and requires PKCE for it (their docs: "we recommend PKCE for
//! confidential apps and require it for public apps"). So ProDeck registers a
//! public app, ships only the client id (not a secret, by design), and proves
//! at the token exchange that the program finishing the flow is the one that
//! started it. Nothing in this file is confidential.
//!
//! Why a loopback redirect. Planning Center sends the browser back to a URL
//! the application registered. ProDeck listens on 127.0.0.1 for exactly one
//! request, for at most five minutes, and only while a sign-in is in flight —
//! it is not a server. That keeps the authorization code on the operator's own
//! machine; it never crosses the internet to prodeck.live or anywhere else.
//! The PAT path is untouched and still works; see `pco::auth`.

use crate::settings::{data_dir, SettingsState};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const AUTHORIZE: &str = "https://api.planningcenteronline.com/oauth/authorize";
const TOKEN: &str = "https://api.planningcenteronline.com/oauth/token";
const REVOKE: &str = "https://api.planningcenteronline.com/oauth/revoke";

/// Only the two products ProDeck reads. `services` is the plan, its items, the
/// team and Services LIVE; `people` is just `people/v2/me`, which is how the
/// app shows who it is connected as. Giving, Check-Ins and the rest are
/// deliberately absent — the consent screen names what it grants, and a church
/// should be able to read that list and not flinch.
const SCOPES: &str = "services people";

/// ProDeck's registered **public** OAuth application. Not a secret: a client id
/// identifies the app, and PKCE (not this string) is what proves the token
/// request is genuine. Empty until the application is registered, and while it
/// is empty the UI asks the church to register their own and paste its id —
/// which is also the escape hatch for anyone who would rather not route
/// through ours.
const BUILT_IN_CLIENT_ID: &str = "";

/// Loopback ports tried in order. Every one of these must be registered as a
/// redirect URI on the OAuth application, so the list is fixed rather than
/// "whatever port the OS hands out" — Planning Center rejects a redirect it
/// has never seen. Three of them because a port can be taken, and one that is
/// busy must not mean "you cannot sign in today". 8088 is missing on purpose:
/// that is the crew gateway's.
const PORTS: &[u16] = &[8127, 8128, 8129];

/// How long the browser has to come back before the listener gives up.
const FLOW_TIMEOUT: Duration = Duration::from_secs(300);

/// Refresh this long before the access token actually dies. Planning Center
/// issues two-hour tokens; a minute of skew costs nothing and keeps a request
/// from racing the expiry.
const REFRESH_SKEW_MS: u64 = 120_000;

const B64URL: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::URL_SAFE_NO_PAD;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Tokens {
    pub access: String,
    pub refresh: String,
    /// Unix ms at which `access` stops working.
    pub expires_at: u64,
    pub scope: String,
    /// Display name of whoever authorized, for "Connected as …".
    pub who: String,
    /// The client id these were issued to. A refresh must be sent with the same
    /// one, so tokens left over from a different application are dead weight
    /// rather than a confusing 401 later.
    pub client_id: String,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ── storage ──────────────────────────────────────────────────────────────
//
// Its own file, not a field in settings.json, for one reason: settings round-trip
// through the browser. The web gateway reads settings out to the Settings page
// and writes the whole object back, and every secret there needs a redact-on-read
// plus a keep-the-old-value-on-write rule to survive that trip. Refresh tokens
// never go near the frontend, so they stay out of the object that does.

fn path() -> PathBuf {
    data_dir().join("pco-oauth.json")
}

struct Cache {
    loaded: bool,
    tokens: Option<Tokens>,
}
static CACHE: Mutex<Cache> = Mutex::new(Cache { loaded: false, tokens: None });

pub(crate) fn load() -> Option<Tokens> {
    let mut g = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    if !g.loaded {
        g.tokens = std::fs::read_to_string(path())
            .ok()
            .and_then(|t| serde_json::from_str::<Tokens>(&t).ok());
        g.loaded = true;
    }
    g.tokens.clone()
}

fn store(t: Option<Tokens>) {
    let p = path();
    match &t {
        Some(t) => {
            let json = serde_json::to_string_pretty(t).unwrap_or_default();
            let _ = crate::settings::write_text_atomic(&p, &json);
            // The file is a credential. Everything else in the data directory is
            // configuration, so this is the one that gets locked down.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
            }
        }
        None => {
            let _ = std::fs::remove_file(&p);
        }
    }
    let mut g = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    g.tokens = t;
    g.loaded = true;
}

/// The app handle, so a refresh failing in the background can tell the UI to
/// ask for a reconnect. Set once at startup; absent in tests.
static APP: OnceLock<AppHandle> = OnceLock::new();

pub fn init(app: AppHandle) {
    let _ = APP.set(app);
}

fn emit(event: &str, payload: serde_json::Value) {
    if let Some(app) = APP.get() {
        let _ = app.emit(event, payload);
    }
}

/// Which OAuth application to use: the church's own if they registered one,
/// otherwise ProDeck's.
pub(crate) fn client_id(settings: &SettingsState) -> Option<String> {
    let own = {
        let s = settings.lock().unwrap_or_else(|p| p.into_inner());
        s.pco_client_id.clone().unwrap_or_default().trim().to_string()
    };
    let id = if own.is_empty() { BUILT_IN_CLIENT_ID.trim() } else { own.as_str() };
    (!id.is_empty()).then(|| id.to_string())
}

// ── PKCE ─────────────────────────────────────────────────────────────────

/// RFC 7636 wants 43–128 characters from the unreserved set. `Uuid::new_v4` is
/// CSPRNG-backed and renders as hex, which is unreserved, so three of them
/// concatenated give far more entropy than required with nothing to escape.
fn verifier() -> String {
    let mut s = String::with_capacity(96);
    for _ in 0..3 {
        s.push_str(&uuid::Uuid::new_v4().simple().to_string());
    }
    s.truncate(64);
    s
}

fn challenge(verifier: &str) -> String {
    let mut h = Sha256::new();
    h.update(verifier.as_bytes());
    B64URL.encode(h.finalize())
}

// ── the flow ─────────────────────────────────────────────────────────────

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

/// Turn Planning Center's OAuth error body into something an operator can act
/// on. These arrive as `{"error":"invalid_client", ...}` with a description
/// written for developers.
fn explain(code: u16, body: &str) -> String {
    let kind = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_default();
    match kind.as_str() {
        "invalid_client" => "Planning Center doesn't recognise this Client ID, or the \
             application is registered as *confidential* rather than *public*. A desktop \
             app has to be public — check the application's type on the developer site."
            .into(),
        "invalid_grant" => "That sign-in has already been used or has expired. Start the \
             connection again."
            .into(),
        "invalid_request" | "invalid_redirect_uri" => format!(
            "Planning Center rejected the redirect address. The OAuth application must list \
             all three of these as redirect URIs:\n{}",
            redirect_uris().join("\n")
        ),
        "access_denied" => "Sign-in was cancelled in the browser.".into(),
        _ if body.trim().is_empty() => format!("Planning Center returned {code} with no detail."),
        _ => format!("Planning Center returned {code}: {}", body.chars().take(300).collect::<String>()),
    }
}

/// The exact strings a church pastes into the OAuth application's redirect
/// field. Shown in the UI and in the error above, so they're generated from
/// `PORTS` rather than written out twice.
pub(crate) fn redirect_uris() -> Vec<String> {
    PORTS.iter().map(|p| format!("http://127.0.0.1:{p}/pco/callback")).collect()
}

#[derive(Serialize)]
pub struct BeginResult {
    /// Opened automatically; also shown so the operator can paste it into a
    /// browser themselves when the default browser isn't the signed-in one.
    pub url: String,
}

#[tauri::command]
pub async fn pco_oauth_begin(
    app: AppHandle,
    settings: tauri::State<'_, SettingsState>,
) -> Result<BeginResult, String> {
    let cid = client_id(&settings).ok_or_else(|| {
        format!(
            "No Planning Center application is configured yet.\n\n\
             A Planning Center **Organization Administrator** can create one at \
             api.planningcenteronline.com → Developers → My Applications → New Application:\n\
             • Type: Public\n\
             • Redirect URIs:\n{}\n\n\
             Then paste its Client ID here. (No secret — a public application doesn't have one.)",
            redirect_uris().join("\n")
        )
    })?;

    // Bind before sending anyone to the browser: discovering the port is busy
    // after Planning Center has already redirected means a dead-end page.
    let (listener, port) = bind().await?;
    let redirect = format!("http://127.0.0.1:{port}/pco/callback");
    let pkce = verifier();
    // A second random value, unrelated to the PKCE one: `state` only has to
    // match on the way back, so that a callback ProDeck didn't start is refused.
    let state = verifier();

    let url = format!(
        "{AUTHORIZE}?client_id={}&redirect_uri={}&response_type=code&scope={}\
         &code_challenge={}&code_challenge_method=S256&state={}",
        urlencoding::encode(&cid),
        urlencoding::encode(&redirect),
        urlencoding::encode(SCOPES),
        urlencoding::encode(&challenge(&pkce)),
        urlencoding::encode(&state),
    );

    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = match wait_for_callback(listener, &state).await {
            Ok(code) => finish(&cid, &code, &pkce, &redirect).await,
            Err(e) => Err(e),
        };
        match result {
            Ok(who) => {
                let _ = app2.emit("pco:oauth", serde_json::json!({ "ok": true, "who": who }));
            }
            Err(e) => {
                let _ = app2.emit("pco:oauth", serde_json::json!({ "ok": false, "error": e }));
            }
        }
    });

    use tauri_plugin_opener::OpenerExt;
    let _ = app.opener().open_url(url.clone(), None::<&str>);
    Ok(BeginResult { url })
}

async fn bind() -> Result<(TcpListener, u16), String> {
    for p in PORTS {
        if let Ok(l) = TcpListener::bind(("127.0.0.1", *p)).await {
            return Ok((l, *p));
        }
    }
    Err(format!(
        "Ports {} are all in use, and those are the only addresses Planning Center is \
         allowed to send the sign-in back to. Quit whatever is using them and try again.",
        PORTS.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ")
    ))
}

/// Accept exactly one request, answer it with a page the operator can close,
/// and hand back the authorization code.
async fn wait_for_callback(listener: TcpListener, want_state: &str) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + FLOW_TIMEOUT;
    loop {
        let accept = tokio::time::timeout_at(deadline, listener.accept())
            .await
            .map_err(|_| "Sign-in timed out — the browser never came back.".to_string())?;
        let (mut sock, _) = accept.map_err(|e| e.to_string())?;

        let mut buf = vec![0u8; 8192];
        let n = sock.read(&mut buf).await.map_err(|e| e.to_string())?;
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        let target = req.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("");

        // Browsers probe /favicon.ico and other paths against any origin they
        // load. Answer and keep waiting rather than treating it as the callback.
        if !target.starts_with("/pco/callback") {
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        }

        let q = parse_query(target);
        let reply = |ok: bool, msg: &str| page(ok, msg);

        if let Some(err) = q.get("error") {
            let msg = if err == "access_denied" {
                "Sign-in was cancelled. Nothing changed."
            } else {
                "Planning Center refused the sign-in."
            };
            let _ = sock.write_all(reply(false, msg).as_bytes()).await;
            return Err(msg.to_string());
        }
        // A mismatched state means this callback was not started by this
        // window — the one thing the parameter exists to catch.
        if q.get("state").map(String::as_str) != Some(want_state) {
            let _ = sock.write_all(reply(false, "This sign-in didn't match the one ProDeck started.").as_bytes()).await;
            return Err("The sign-in didn't match the one ProDeck started. Try connecting again.".into());
        }
        let code = match q.get("code") {
            Some(c) if !c.is_empty() => c.clone(),
            _ => {
                let _ = sock.write_all(reply(false, "Planning Center sent no authorization code.").as_bytes()).await;
                return Err("Planning Center sent no authorization code.".into());
            }
        };
        let _ = sock
            .write_all(reply(true, "ProDeck is connected to Planning Center. You can close this tab.").as_bytes())
            .await;
        let _ = sock.flush().await;
        return Ok(code);
    }
}

fn parse_query(target: &str) -> std::collections::HashMap<String, String> {
    target
        .split_once('?')
        .map(|(_, q)| q)
        .unwrap_or("")
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| {
            (
                urlencoding::decode(k).unwrap_or_default().to_string(),
                urlencoding::decode(v).unwrap_or_default().to_string(),
            )
        })
        .collect()
}

/// The browser tab the operator is left looking at. Self-contained: it is served
/// from a socket that closes immediately after, so it can't fetch anything.
fn page(ok: bool, msg: &str) -> String {
    let accent = if ok { "#1f9d55" } else { "#b4332a" };
    let title = if ok { "Connected" } else { "Not connected" };
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>ProDeck — {title}</title>\
         <style>:root{{color-scheme:light dark}}body{{margin:0;min-height:100vh;display:grid;\
         place-items:center;font:16px/1.5 -apple-system,Segoe UI,system-ui,sans-serif;\
         background:Canvas;color:CanvasText}}div{{max-width:28rem;padding:2rem;text-align:center}}\
         h1{{margin:0 0 .5rem;font-size:1.25rem;color:{accent}}}p{{margin:0;opacity:.75}}</style>\
         <div><h1>{title}</h1><p>{}</p></div>",
        html_escape(msg)
    );
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Exchange the code, learn who signed in, and save. Returns the display name.
async fn finish(client_id: &str, code: &str, verifier: &str, redirect: &str) -> Result<String, String> {
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("code_verifier", verifier),
        ("client_id", client_id),
        ("redirect_uri", redirect),
    ];
    let mut t = post_token(&form, client_id).await?;
    t.who = whoami(&t.access).await.unwrap_or_default();
    let who = t.who.clone();
    store(Some(t));
    Ok(who)
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
    #[serde(default)]
    scope: String,
}

async fn post_token(form: &[(&str, &str)], client_id: &str) -> Result<Tokens, String> {
    let resp = http_client()?
        .post(TOKEN)
        .header("User-Agent", "ProDeck")
        .form(form)
        .send()
        .await
        .map_err(|e| format!("Couldn't reach Planning Center: {e}"))?;
    let code = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    if !(200..300).contains(&code) {
        return Err(explain(code, &body));
    }
    let r: TokenResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Planning Center's reply didn't parse: {e}"))?;
    Ok(Tokens {
        access: r.access_token,
        refresh: r.refresh_token,
        expires_at: now_ms() + r.expires_in.saturating_mul(1000),
        scope: r.scope,
        who: String::new(),
        client_id: client_id.to_string(),
    })
}

async fn whoami(access: &str) -> Option<String> {
    let v: serde_json::Value = http_client()
        .ok()?
        .get("https://api.planningcenteronline.com/people/v2/me")
        .bearer_auth(access)
        .header("User-Agent", "ProDeck")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    v.pointer("/data/attributes/name")?.as_str().map(str::to_string)
}

// ── keeping it alive ─────────────────────────────────────────────────────

/// One refresh at a time. The plan sync, the live poll and a UI action can all
/// notice the expiry in the same second; without this they would each spend the
/// refresh token, and Planning Center rotates it — the first exchange wins and
/// the others invalidate the credential they were trying to renew.
static REFRESHING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A usable bearer token, refreshing first if the current one is about to die.
/// `None` means no OAuth connection — callers fall back to the token pair.
pub(crate) async fn access_token(settings: &SettingsState) -> Option<String> {
    let t = load()?;
    // A church that switched to their own OAuth application has tokens issued
    // by the old one. They cannot be refreshed against the new client id, so
    // treat them as absent rather than failing every request with a 401.
    let cid = client_id(settings)?;
    if t.client_id != cid {
        return None;
    }
    if now_ms() + REFRESH_SKEW_MS < t.expires_at {
        return Some(t.access);
    }
    refresh(&cid).await
}

/// Force a renewal regardless of the recorded expiry — used when a request came
/// back 401 despite the clock saying the token was fine (clock skew, or the
/// token revoked and reissued elsewhere).
pub(crate) async fn refresh_now(settings: &SettingsState) -> Option<String> {
    let cid = client_id(settings)?;
    let before = load()?.access;
    let t = refresh(&cid).await?;
    // If another task already renewed while we waited on the lock, that new
    // token is the fresh one and there is nothing more to do.
    (t != before).then_some(t)
}

async fn refresh(client_id: &str) -> Option<String> {
    let _g = REFRESHING.lock().await;
    // Re-read under the lock: whoever held it before us may have just renewed.
    let t = load()?;
    if now_ms() + REFRESH_SKEW_MS < t.expires_at {
        return Some(t.access);
    }
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", t.refresh.as_str()),
        ("client_id", client_id),
    ];
    match post_token(&form, client_id).await {
        Ok(mut new) => {
            // The refresh response has no name on it; keep the one we learned
            // at sign-in so the UI doesn't blank out every two hours.
            new.who = t.who;
            let access = new.access.clone();
            store(Some(new));
            Some(access)
        }
        Err(e) => {
            // Planning Center honours a refresh token for 90 days. Past that —
            // or after the church revokes ProDeck from their account — it is
            // gone for good, and silently retrying forever would leave the
            // operator staring at an empty plan with no idea why.
            store(None);
            emit("pco:oauth", serde_json::json!({ "ok": false, "error": e, "reconnect": true }));
            None
        }
    }
}

// ── commands ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct OauthStatus {
    /// True once tokens exist for the configured application.
    pub connected: bool,
    pub who: String,
    pub scope: String,
    /// True when an OAuth application is configured at all — the Connect button
    /// is only meaningful then.
    pub configured: bool,
    /// Whether the client id came from the church's own settings.
    pub own_app: bool,
    pub redirect_uris: Vec<String>,
}

#[tauri::command]
pub fn pco_oauth_status(settings: tauri::State<'_, SettingsState>) -> OauthStatus {
    let cid = client_id(&settings);
    let own_app = {
        let s = settings.lock().unwrap_or_else(|p| p.into_inner());
        !s.pco_client_id.clone().unwrap_or_default().trim().is_empty()
    };
    let t = load().filter(|t| Some(&t.client_id) == cid.as_ref());
    OauthStatus {
        connected: t.is_some(),
        who: t.as_ref().map(|t| t.who.clone()).unwrap_or_default(),
        scope: t.as_ref().map(|t| t.scope.clone()).unwrap_or_default(),
        configured: cid.is_some(),
        own_app,
        redirect_uris: redirect_uris(),
    }
}

/// Disconnect: tell Planning Center to drop the token, then forget it here.
/// Revoking first matters — a token we merely forgot would stay live on their
/// side for up to two hours and keep showing in the church's connected apps.
#[tauri::command]
pub async fn pco_oauth_disconnect(settings: tauri::State<'_, SettingsState>) -> Result<(), String> {
    let (t, cid) = (load(), client_id(&settings));
    if let (Some(t), Some(cid)) = (t, cid) {
        if let Ok(c) = http_client() {
            let _ = c
                .post(REVOKE)
                .header("User-Agent", "ProDeck")
                .form(&[
                    ("token", t.refresh.as_str()),
                    ("token_type_hint", "refresh_token"),
                    ("client_id", cid.as_str()),
                ])
                .send()
                .await;
        }
    }
    store(None);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_matches_rfc7636() {
        // The worked example from RFC 7636 appendix B. If this drifts, every
        // token exchange fails with invalid_grant and nothing says why.
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(challenge(v), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn verifiers_are_unreserved_and_long_enough() {
        let v = verifier();
        assert!((43..=128).contains(&v.len()), "len {}", v.len());
        assert!(v.chars().all(|c| c.is_ascii_alphanumeric() || "-._~".contains(c)), "{v}");
        assert_ne!(verifier(), verifier(), "two flows must not share a verifier");
    }

    #[test]
    fn redirect_uris_are_loopback_only() {
        // A redirect that isn't 127.0.0.1 would send the authorization code off
        // the machine. Registered URIs must match these exactly, so this is the
        // list the UI shows and the list PCO is given.
        let uris = redirect_uris();
        assert_eq!(uris.len(), PORTS.len());
        assert!(uris.iter().all(|u| u.starts_with("http://127.0.0.1:")), "{uris:?}");
        assert!(uris.iter().all(|u| u.ends_with("/pco/callback")), "{uris:?}");
        // 8088 is the crew gateway; binding it here would fight the web server.
        assert!(!PORTS.contains(&8088));
    }

    #[test]
    fn callback_query_is_parsed_and_decoded() {
        let q = parse_query("/pco/callback?code=ab%2Fcd&state=xyz");
        assert_eq!(q.get("code").map(String::as_str), Some("ab/cd"));
        assert_eq!(q.get("state").map(String::as_str), Some("xyz"));
        assert!(parse_query("/pco/callback").is_empty());
    }

    #[test]
    fn scopes_stay_minimal() {
        // Widening this widens the consent screen every church reads. Adding a
        // product here should be a deliberate edit, not a drive-by.
        let mut s: Vec<&str> = SCOPES.split_whitespace().collect();
        s.sort();
        assert_eq!(s, vec!["people", "services"]);
    }

    /// Drive a real loopback round-trip: bind, send the callback Planning
    /// Center would send, and check what comes back out. The listener is the
    /// one part of this flow that can't be reasoned about from the types — it
    /// parses a raw socket — and it is also the part that, if it accepted the
    /// wrong request, would hand an attacker's authorization code to ProDeck.
    async fn round_trip(query: &str) -> Result<String, String> {
        let l = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = l.local_addr().unwrap();
        let q = query.to_string();
        let client = tokio::spawn(async move {
            let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
            s.write_all(format!("GET {q} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
                .await
                .unwrap();
            let mut out = String::new();
            let _ = tokio::io::AsyncReadExt::read_to_string(&mut s, &mut out).await;
            out
        });
        let got = wait_for_callback(l, "st4te").await;
        let page = client.await.unwrap();
        // Whatever happened, the operator is left looking at a real page and
        // not a hung tab or a connection reset.
        assert!(page.starts_with("HTTP/1.1 200 OK"), "{page}");
        assert!(page.contains("<title>ProDeck"), "{page}");
        got
    }

    #[tokio::test]
    async fn the_callback_hands_back_the_code() {
        assert_eq!(round_trip("/pco/callback?code=abc123&state=st4te").await, Ok("abc123".into()));
    }

    #[tokio::test]
    async fn a_callback_we_didnt_start_is_refused() {
        // The entire job of the `state` parameter. Without this check, anything
        // that can reach 127.0.0.1 could feed ProDeck an authorization code
        // from a different Planning Center account.
        let e = round_trip("/pco/callback?code=abc123&state=somebody-elses").await.unwrap_err();
        assert!(e.contains("didn't match"), "{e}");
    }

    #[tokio::test]
    async fn a_refusal_in_the_browser_comes_back_as_cancelled() {
        let e = round_trip("/pco/callback?error=access_denied&state=st4te").await.unwrap_err();
        assert!(e.contains("cancelled"), "{e}");
    }

    #[tokio::test]
    async fn a_callback_with_no_code_is_not_treated_as_success() {
        let e = round_trip("/pco/callback?state=st4te").await.unwrap_err();
        assert!(e.contains("no authorization code"), "{e}");
    }

    #[tokio::test]
    async fn stray_requests_dont_end_the_wait() {
        // Browsers fetch /favicon.ico against any origin they load. Treating
        // that as the callback would abort the sign-in a second after it
        // started, every time, on some browsers and not others.
        let l = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            for req in ["/favicon.ico", "/", "/pco/callback?code=real&state=st4te"] {
                let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
                s.write_all(format!("GET {req} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
                    .await
                    .unwrap();
                let mut out = String::new();
                let _ = tokio::io::AsyncReadExt::read_to_string(&mut s, &mut out).await;
            }
        });
        assert_eq!(wait_for_callback(l, "st4te").await, Ok("real".into()));
    }

    #[test]
    fn oauth_errors_name_the_fix() {
        let m = explain(401, r#"{"error":"invalid_client"}"#);
        assert!(m.contains("public"), "{m}");
        let m = explain(400, r#"{"error":"invalid_redirect_uri"}"#);
        assert!(m.contains("127.0.0.1:8127"), "{m}");
        // An unrecognised shape still shows what came back rather than eating it.
        assert!(explain(500, "upstream boom").contains("upstream boom"));
    }

}
