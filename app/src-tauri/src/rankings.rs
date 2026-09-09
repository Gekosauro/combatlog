//! Warcraft Logs OAuth (PKCE) and per-report percentile retrieval.

use std::time::Duration;

use anyhow::{anyhow, bail, Context as _, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::{distributions::Alphanumeric, Rng as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use tauri_plugin_opener::OpenerExt as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use url::Url;
use wreq::Client;
use wreq_util::Emulation;

const BASE_URL: &str = "https://www.warcraftlogs.com";
pub const REDIRECT_URI: &str = "http://127.0.0.1:48173/oauth/callback";
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankingsConfig {
    pub client_id: Option<&'static str>,
    pub redirect_uri: &'static str,
}

pub fn config() -> RankingsConfig {
    RankingsConfig {
        client_id: option_env!("WCL_CLIENT_ID").filter(|value| !value.trim().is_empty()),
        redirect_uri: REDIRECT_URI,
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthToken {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    #[serde(default = "default_token_type")]
    token_type: String,
    #[serde(default = "default_expires_in")]
    expires_in: u64,
}

fn default_token_type() -> String {
    "Bearer".to_string()
}
fn default_expires_in() -> u64 {
    3600
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReportPercentiles {
    pub parse_percent: Option<u32>,
    pub bracket_percent: Option<u32>,
    pub fights_ranked: usize,
    pub role: Option<String>,
    pub pending: bool,
}

fn random_url_safe(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

fn authorization_url(client_id: &str, state: &str, challenge: &str) -> Result<Url> {
    let mut url = Url::parse(&format!("{BASE_URL}/oauth/authorize"))?;
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("response_type", "code")
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(url)
}

/// Opens the official Warcraft Logs consent page and catches its loopback redirect.
pub async fn authorize(app: &tauri::AppHandle, client_id: &str) -> Result<OAuthToken> {
    let client_id = client_id.trim();
    if client_id.is_empty() {
        bail!("A Warcraft Logs API Client ID is required.");
    }

    // Bind before opening the browser so even an immediate redirect cannot be lost.
    let listener = TcpListener::bind("127.0.0.1:48173")
        .await
        .context("starting the local Warcraft Logs authorization callback")?;
    let verifier = random_url_safe(96);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_url_safe(40);
    let url = authorization_url(client_id, &state, &challenge)?;
    app.opener()
        .open_url(url.as_str(), None::<String>)
        .context("opening the Warcraft Logs authorization page")?;

    let code = tokio::time::timeout(CALLBACK_TIMEOUT, receive_code(listener, &state))
        .await
        .map_err(|_| anyhow!("Warcraft Logs authorization timed out. Try connecting again."))??;
    exchange_code(client_id, &verifier, &code).await
}

async fn receive_code(listener: TcpListener, expected_state: &str) -> Result<String> {
    loop {
        let (mut socket, _) = listener.accept().await?;
        let mut request = vec![0u8; 16 * 1024];
        let size = socket.read(&mut request).await?;
        let request = String::from_utf8_lossy(&request[..size]);
        let target = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/");
        let callback = Url::parse(&format!("http://127.0.0.1:48173{target}"))?;
        if callback.path() != "/oauth/callback" {
            respond(&mut socket, 404, "Authorization callback not found.").await?;
            continue;
        }
        let pairs = callback
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        if pairs.get("state").map(|v| v.as_ref()) != Some(expected_state) {
            respond(
                &mut socket,
                400,
                "The authorization state did not match. Return to combatlog.dev and try again.",
            )
            .await?;
            bail!("Warcraft Logs returned an invalid authorization state.");
        }
        if let Some(error) = pairs.get("error") {
            respond(
                &mut socket,
                400,
                "Warcraft Logs authorization was cancelled. You can close this tab.",
            )
            .await?;
            bail!("Warcraft Logs authorization failed: {error}");
        }
        let code = pairs
            .get("code")
            .map(|value| value.to_string())
            .context("Warcraft Logs did not return an authorization code")?;
        respond(&mut socket, 200, "Warcraft Logs is connected to combatlog.dev. You can close this tab and return to the app.").await?;
        return Ok(code);
    }
}

async fn respond(socket: &mut tokio::net::TcpStream, status: u16, message: &str) -> Result<()> {
    let label = if status == 200 { "OK" } else { "Error" };
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>combatlog.dev</title>\
         <style>body{{font:16px system-ui;background:#101014;color:#eee;display:grid;place-items:center;min-height:100vh;margin:0}}\
         main{{max-width:520px;padding:32px;border:1px solid #4d3479;border-radius:16px;background:#18151f}}h1{{color:#ad80fb}}</style>\
         <main><h1>{label}</h1><p>{message}</p></main>"
    );
    let response = format!(
        "HTTP/1.1 {status} {label}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(response.as_bytes()).await?;
    socket.shutdown().await?;
    Ok(())
}

async fn exchange_code(client_id: &str, verifier: &str, code: &str) -> Result<OAuthToken> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("grant_type", "authorization_code")
        .append_pair("code", code)
        .append_pair("code_verifier", verifier)
        .finish();
    let response = http_client()?
        .post(format!("{BASE_URL}/oauth/token"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .context("requesting a Warcraft Logs access token")?;
    let status = response.status().as_u16();
    let text = response.text().await?;
    if status >= 400 {
        bail!(
            "Warcraft Logs token request failed (HTTP {status}): {}",
            truncate(&text, 300)
        );
    }
    let token: OAuthTokenResponse =
        serde_json::from_str(&text).context("reading the Warcraft Logs access token")?;
    Ok(OAuthToken {
        access_token: token.access_token,
        token_type: token.token_type,
        expires_in: token.expires_in,
    })
}

fn http_client() -> Result<Client> {
    Ok(Client::builder().emulation(Emulation::Chrome133).build()?)
}

pub async fn fetch_report(
    access_token: &str,
    report_code: &str,
    character: &str,
) -> Result<ReportPercentiles> {
    if access_token.trim().is_empty() {
        bail!("Connect Warcraft Logs rankings first.");
    }
    let code = report_code.trim();
    if code.is_empty() || !code.chars().all(|c| c.is_ascii_alphanumeric()) {
        bail!("Invalid Warcraft Logs report code.");
    }
    let query = r#"
      query CombatlogReportRankings($code: String!) {
        reportData {
          report(code: $code) {
            rankingsDps: rankings(playerMetric: dps)
            rankingsHps: rankings(playerMetric: hps)
          }
        }
      }
    "#;
    let response = http_client()?
        .post(format!("{BASE_URL}/api/v2/user"))
        .bearer_auth(access_token.trim())
        .json(&json!({"query": query, "variables": {"code": code}}))
        .send()
        .await
        .context("requesting Warcraft Logs rankings")?;
    let status = response.status().as_u16();
    let text = response.text().await?;
    if status == 401 || status == 403 {
        bail!("Warcraft Logs authorization expired. Connect rankings again.");
    }
    if status >= 400 {
        bail!(
            "Warcraft Logs rankings request failed (HTTP {status}): {}",
            truncate(&text, 300)
        );
    }
    let payload: Value = serde_json::from_str(&text).context("reading Warcraft Logs rankings")?;
    if let Some(errors) = payload.get("errors").and_then(Value::as_array) {
        let message = errors
            .iter()
            .filter_map(|item| item.get("message").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("; ");
        bail!("Warcraft Logs rankings query failed: {message}");
    }
    let report = payload
        .pointer("/data/reportData/report")
        .context("Warcraft Logs report is not available to this account")?;
    Ok(extract_percentiles(
        report.get("rankingsDps"),
        report.get("rankingsHps"),
        character,
    ))
}

#[derive(Clone)]
struct CharacterRef {
    name: String,
    realm: Option<String>,
}

fn selected_character(value: &str) -> CharacterRef {
    let mut parts = value.trim().split('-').collect::<Vec<_>>();
    if parts.last().is_some_and(|part| {
        matches!(
            part.to_ascii_uppercase().as_str(),
            "US" | "EU" | "KR" | "TW" | "CN"
        )
    }) {
        parts.pop();
    }
    CharacterRef {
        name: parts.first().copied().unwrap_or("").trim().to_string(),
        realm: (parts.len() > 1).then(|| parts[1..].join("-")),
    }
}

#[derive(Clone, Copy)]
enum MetricRole {
    Damage,
    Healing,
}

fn ranking_rows<'a>(payload: Option<&'a Value>, preferred: MetricRole) -> Vec<&'a Value> {
    let mut out = Vec::new();
    let Some(data) = payload
        .and_then(|value| value.get("data"))
        .and_then(Value::as_array)
    else {
        return out;
    };
    let roles: &[&str] = match preferred {
        MetricRole::Damage => &["dps", "tanks"],
        MetricRole::Healing => &["healers"],
    };
    for fight in data {
        for role in roles {
            if let Some(characters) = fight
                .pointer(&format!("/roles/{role}/characters"))
                .and_then(Value::as_array)
            {
                out.extend(characters);
            }
        }
    }
    out
}

fn character_matches(row: &Value, selected: &CharacterRef) -> bool {
    let Some(name) = row.get("name").and_then(Value::as_str) else {
        return false;
    };
    if !name.eq_ignore_ascii_case(&selected.name) {
        return false;
    }
    let Some(expected_realm) = selected.realm.as_deref() else {
        return true;
    };
    let actual_realm = row
        .pointer("/server/name")
        .or_else(|| row.pointer("/server/slug"))
        .or_else(|| row.get("serverName"))
        .and_then(Value::as_str);
    actual_realm.is_none_or(|realm| normalize(realm) == normalize(expected_realm))
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn mean_percent(rows: &[&Value], field: &str) -> Option<u32> {
    let values = rows
        .iter()
        .filter_map(|row| row.get(field).and_then(Value::as_f64))
        .filter(|value| value.is_finite() && *value >= 0.0 && *value <= 100.0)
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| (values.iter().sum::<f64>() / values.len() as f64).round() as u32)
}

fn extract_percentiles(
    dps: Option<&Value>,
    hps: Option<&Value>,
    character: &str,
) -> ReportPercentiles {
    let selected = selected_character(character);
    if selected.name.is_empty() {
        return ReportPercentiles {
            parse_percent: None,
            bracket_percent: None,
            fights_ranked: 0,
            role: None,
            pending: true,
        };
    }
    let healing = ranking_rows(hps, MetricRole::Healing)
        .into_iter()
        .filter(|row| character_matches(row, &selected))
        .collect::<Vec<_>>();
    let (rows, role) = if healing.is_empty() {
        (
            ranking_rows(dps, MetricRole::Damage)
                .into_iter()
                .filter(|row| character_matches(row, &selected))
                .collect::<Vec<_>>(),
            "damage",
        )
    } else {
        (healing, "healing")
    };
    let parse_percent = mean_percent(&rows, "rankPercent");
    let bracket_percent = mean_percent(&rows, "bracketPercent");
    ReportPercentiles {
        parse_percent,
        bracket_percent,
        fights_ranked: rows.len(),
        role: (!rows.is_empty()).then(|| role.to_string()),
        pending: parse_percent.is_none() && bracket_percent.is_none(),
    }
}

fn truncate(value: &str, max: usize) -> String {
    let mut chars = value.chars();
    let prefix = chars.by_ref().take(max).collect::<String>();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rankings(role: &str, characters: Value) -> Value {
        let mut roles = serde_json::Map::new();
        roles.insert(role.to_string(), json!({"characters": characters}));
        json!({"data": [{"roles": Value::Object(roles)}]})
    }

    #[test]
    fn selects_character_and_averages_ranked_raid_fights() {
        let dps = json!({"data": [
            {"roles": {"dps": {"characters": [
                {"name":"Other","server":{"name":"Nemesis"},"rankPercent":99,"bracketPercent":98},
                {"name":"Geko","server":{"name":"Nemesis"},"rankPercent":70,"bracketPercent":60}
            ]}}},
            {"roles": {"dps": {"characters": [
                {"name":"Geko","server":{"name":"Nemesis"},"rankPercent":80,"bracketPercent":90}
            ]}}}
        ]});
        let result = extract_percentiles(Some(&dps), None, "Geko-Nemesis-EU");
        assert_eq!(result.parse_percent, Some(75));
        assert_eq!(result.bracket_percent, Some(75));
        assert_eq!(result.fights_ranked, 2);
        assert_eq!(result.role.as_deref(), Some("damage"));
        assert!(!result.pending);
    }

    #[test]
    fn healer_uses_healing_ranking_instead_of_damage_ranking() {
        let dps = rankings(
            "dps",
            json!([{"name":"Heals","rankPercent":12,"bracketPercent":10}]),
        );
        let hps = rankings(
            "healers",
            json!([{"name":"Heals","rankPercent":88,"bracketPercent":91}]),
        );
        let result = extract_percentiles(Some(&dps), Some(&hps), "Heals");
        assert_eq!(result.parse_percent, Some(88));
        assert_eq!(result.bracket_percent, Some(91));
        assert_eq!(result.role.as_deref(), Some("healing"));
    }

    #[test]
    fn missing_or_unprocessed_rankings_are_pending() {
        let result = extract_percentiles(None, None, "Geko-Nemesis-EU");
        assert_eq!(result.parse_percent, None);
        assert_eq!(result.bracket_percent, None);
        assert_eq!(result.fights_ranked, 0);
        assert!(result.pending);
    }

    #[test]
    fn oauth_url_uses_pkce_and_loopback_callback() {
        let url = authorization_url("client-123", "state-456", "challenge-789").unwrap();
        let query = url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            query.get("client_id").map(|v| v.as_ref()),
            Some("client-123")
        );
        assert_eq!(
            query.get("redirect_uri").map(|v| v.as_ref()),
            Some(REDIRECT_URI)
        );
        assert_eq!(
            query.get("code_challenge_method").map(|v| v.as_ref()),
            Some("S256")
        );
    }
}
