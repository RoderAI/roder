//! OAuth2 service-account token minting for Vertex AI (roadmap phase 92).
//!
//! Signs an RS256 JWT-bearer assertion with the service-account private key
//! and exchanges it at the OAuth token endpoint for a ~1h access token.
//! Credentials come from `GOOGLE_APPLICATION_CREDENTIALS` (file path) or
//! inline `VERTEX_CREDENTIALS_JSON`. Tokens are cached and refreshed 60
//! seconds before expiry. Secrets never appear in errors or `Debug` output.

use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::json;

pub const CREDENTIALS_PATH_ENV: &str = "GOOGLE_APPLICATION_CREDENTIALS";
pub const CREDENTIALS_JSON_ENV: &str = "VERTEX_CREDENTIALS_JSON";

const DEFAULT_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const EXPIRY_SLACK: Duration = Duration::from_secs(60);
/// Tolerates clock skew between this host and the token endpoint.
const IAT_SKEW: Duration = Duration::from_secs(10);
const TOKEN_LIFETIME: Duration = Duration::from_secs(3600);

pub fn missing_credentials_error() -> anyhow::Error {
    anyhow::anyhow!(
        "Vertex AI credentials are missing; set {CREDENTIALS_PATH_ENV} to a service-account \
         JSON file or {CREDENTIALS_JSON_ENV} to inline service-account JSON"
    )
}

#[derive(Clone, Deserialize)]
pub struct ServiceAccountCredentials {
    #[serde(rename = "type")]
    kind: String,
    pub client_email: String,
    private_key: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    token_uri: Option<String>,
}

impl std::fmt::Debug for ServiceAccountCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceAccountCredentials")
            .field("kind", &self.kind)
            .field("client_email", &self.client_email)
            .field("private_key", &"<redacted>")
            .field("project_id", &self.project_id)
            .field("token_uri", &self.token_uri)
            .finish()
    }
}

impl ServiceAccountCredentials {
    /**
     * Parses service-account JSON from the inline value or the file at
     * `credentials_path` (inline wins). `Ok(None)` when neither is set;
     * `Err` when a source is set but unreadable or not a service account.
     */
    pub fn resolve(
        credentials_json: Option<&str>,
        credentials_path: Option<&str>,
    ) -> anyhow::Result<Option<Self>> {
        let (text, source) = if let Some(inline) = credentials_json {
            (inline.to_string(), CREDENTIALS_JSON_ENV.to_string())
        } else if let Some(path) = credentials_path {
            let text = std::fs::read_to_string(path).map_err(|error| {
                anyhow::anyhow!("failed to read {CREDENTIALS_PATH_ENV} file: {error}")
            })?;
            (text, format!("{CREDENTIALS_PATH_ENV} file"))
        } else {
            return Ok(None);
        };
        let credentials: Self = serde_json::from_str(&text)
            .map_err(|_| anyhow::anyhow!("{source} is not valid service-account JSON"))?;
        anyhow::ensure!(
            credentials.kind == "service_account",
            "{source} has credential type {:?}; Vertex AI needs a service_account key",
            credentials.kind
        );
        anyhow::ensure!(
            !credentials.client_email.is_empty() && !credentials.private_key.is_empty(),
            "{source} is missing client_email or private_key"
        );
        Ok(Some(credentials))
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
}

struct CachedToken {
    token: String,
    expires_at: Instant,
}

/// Cached service-account token source.
pub struct ServiceAccountTokenSource {
    credentials_json: Option<String>,
    credentials_path: Option<String>,
    /// Overrides both the JWT `aud` claim and the exchange URL (fake tests).
    token_endpoint_override: Option<String>,
    cache: Mutex<Option<CachedToken>>,
}

impl ServiceAccountTokenSource {
    pub fn new(
        credentials_json: Option<String>,
        credentials_path: Option<String>,
        token_endpoint_override: Option<String>,
    ) -> Self {
        Self {
            credentials_json,
            credentials_path,
            token_endpoint_override,
            cache: Mutex::new(None),
        }
    }

    /// Whether a credential source looks present without minting (used for
    /// `auth_configured` hints; never performs network I/O).
    pub fn looks_configured(&self) -> bool {
        self.credentials_json.is_some()
            || self
                .credentials_path
                .as_ref()
                .is_some_and(|path| std::path::Path::new(path).is_file())
    }

    /// Project id from the credentials JSON, used when `VERTEX_PROJECT` is
    /// not set.
    pub fn project_from_credentials(&self) -> Option<String> {
        ServiceAccountCredentials::resolve(
            self.credentials_json.as_deref(),
            self.credentials_path.as_deref(),
        )
        .ok()
        .flatten()
        .and_then(|credentials| credentials.project_id)
    }

    /// Returns a valid access token, minting when the cache is empty or
    /// within the expiry slack.
    pub async fn access_token(&self) -> anyhow::Result<String> {
        if let Some(cached) = self.cache.lock().unwrap().as_ref()
            && Instant::now() + EXPIRY_SLACK < cached.expires_at
        {
            return Ok(cached.token.clone());
        }
        let credentials = ServiceAccountCredentials::resolve(
            self.credentials_json.as_deref(),
            self.credentials_path.as_deref(),
        )?
        .ok_or_else(missing_credentials_error)?;
        let (token, expires_in) = self.mint(&credentials).await?;
        let expires_at = Instant::now() + Duration::from_secs(expires_in.unwrap_or(3600));
        *self.cache.lock().unwrap() = Some(CachedToken {
            token: token.clone(),
            expires_at,
        });
        Ok(token)
    }

    fn token_endpoint<'a>(&'a self, credentials: &'a ServiceAccountCredentials) -> &'a str {
        if let Some(endpoint) = self.token_endpoint_override.as_deref() {
            return endpoint;
        }
        credentials
            .token_uri
            .as_deref()
            .filter(|uri| !uri.is_empty())
            .unwrap_or(DEFAULT_TOKEN_ENDPOINT)
    }

    async fn mint(
        &self,
        credentials: &ServiceAccountCredentials,
    ) -> anyhow::Result<(String, Option<u64>)> {
        let endpoint = self.token_endpoint(credentials);
        let assertion = signed_jwt_assertion(credentials, endpoint, unix_now())?;
        let response = reqwest::Client::new()
            .post(endpoint)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            // Surface only the OAuth error code; bodies can carry credential
            // hints and must never be echoed.
            let code = response
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|body| body.get("error")?.as_str().map(str::to_string));
            anyhow::bail!(token_exchange_error(status.as_u16(), code.as_deref()));
        }
        let token: TokenResponse = response.json().await?;
        anyhow::ensure!(
            !token.access_token.is_empty(),
            "Vertex AI token endpoint returned an empty access token"
        );
        Ok((token.access_token, token.expires_in))
    }
}

fn token_exchange_error(status: u16, code: Option<&str>) -> String {
    let hint = match code {
        Some("invalid_grant") => {
            "; check the host clock and that the service-account key is not revoked or deleted"
        }
        Some("access_denied") => {
            "; grant the service account access to Vertex AI (e.g. roles/aiplatform.user)"
        }
        _ => "",
    };
    match code {
        Some(code) => {
            format!("Vertex AI token exchange failed with {status} ({code}){hint}")
        }
        None => format!("Vertex AI token exchange failed with {status}"),
    }
}

/// Builds and RS256-signs the JWT-bearer assertion for the token grant.
fn signed_jwt_assertion(
    credentials: &ServiceAccountCredentials,
    audience: &str,
    issued_at: u64,
) -> anyhow::Result<String> {
    let header = URL_SAFE_NO_PAD.encode(json!({ "alg": "RS256", "typ": "JWT" }).to_string());
    let iat = issued_at.saturating_sub(IAT_SKEW.as_secs());
    let claims = URL_SAFE_NO_PAD.encode(
        json!({
            "iss": credentials.client_email,
            "scope": CLOUD_PLATFORM_SCOPE,
            "aud": audience,
            "iat": iat,
            "exp": iat + TOKEN_LIFETIME.as_secs(),
        })
        .to_string(),
    );
    let message = format!("{header}.{claims}");
    let signature = rs256_sign(&credentials.private_key, message.as_bytes())?;
    Ok(format!("{message}.{}", URL_SAFE_NO_PAD.encode(signature)))
}

fn rs256_sign(private_key_pem: &str, message: &[u8]) -> anyhow::Result<Vec<u8>> {
    let der = pkcs8_der_from_pem(private_key_pem)?;
    let key_pair = ring::signature::RsaKeyPair::from_pkcs8(&der)
        .map_err(|_| anyhow::anyhow!("service-account private_key is not a usable RSA key"))?;
    let mut signature = vec![0_u8; key_pair.public().modulus_len()];
    key_pair
        .sign(
            &ring::signature::RSA_PKCS1_SHA256,
            &ring::rand::SystemRandom::new(),
            message,
            &mut signature,
        )
        .map_err(|_| anyhow::anyhow!("RS256 signing with the service-account key failed"))?;
    Ok(signature)
}

fn pkcs8_der_from_pem(pem: &str) -> anyhow::Result<Vec<u8>> {
    let mut reader = std::io::BufReader::new(pem.as_bytes());
    rustls_pemfile::pkcs8_private_keys(&mut reader)
        .next()
        .and_then(Result::ok)
        .map(|key| key.secret_pkcs8_der().to_vec())
        .ok_or_else(|| {
            anyhow::anyhow!("service-account private_key is not a PKCS#8 PEM private key")
        })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
pub(crate) mod test_credentials {
    use serde_json::json;

    /**
     * Throwaway 2048-bit RSA key generated solely for these tests; it is not
     * a credential for any real system.
     */
    pub(crate) const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQDRckx2V5OCi63p\n\
6Rya2ViCt8hbWmKzrdsybLRQHaDRr/1CfAAQJ2lHrpWUMsz2Dzj2icLKqZ5VidJ6\n\
ttxLMpJbhWsHfU98OU4oHNJc9UCfeAZc+ayEflWdFxLTHjV7h/w1hG4gr10PSWqS\n\
Sy4d4IThjm/wnqJ8FLOUyrJgx7KX6KJv3e1Xkqc2KE+z5BFdl8nCTP80+MxZ5K/H\n\
/RIYCFq5k3zPrDeGg5NQFLOP7MMO6bO1bJOrCDaKVhJMVvepwqtKLE3nO0Imyl+D\n\
DYz5uCGvKYh3HBS4E585f7+gwaqOIR8QrSCDbHU2BCfo5ExfiAGGRk9JgiCAH4Ny\n\
BThgzQmfAgMBAAECggEBAJu/8mpCf7ghZMfACPyBydcTEdQVJ7bT/1/FBGVbUv77\n\
b0rkaSuaEykyA5t8F3yXH1X+ZbNNZSfY4INOvgzRY5LZaRjdr6ECAEPGAw0Ld+3e\n\
VGUJaafxRnsV8HK8USs2mW+2tipqHbrDbpOxgm7HSiltQYLehJfe0RhBj1p2xjE9\n\
fjBh8I3m52pKvavY29tIGX9l7QD3tVOHk0AkVrPRLZiZx5pW9xRJMkX0t74gIAT+\n\
fr/bNBxCwiLe3FYTZGXVL3ohPLJClGUijI8QMbXhNxZH9yZyVYRbEJ5SLg3bEIxo\n\
hnmgiKvF/H5ceZpwZ1avtsDAjazMJ3jjpDZ8uvuru0ECgYEA7E8JlKsGZThH0/l2\n\
PHqXD642q5NKTNY/8MniAdfQgBm7pmCInZEfUDR+BBHOaljpYRadYZJLwccQe32H\n\
ZvQr65ExOg/HZff647ViJ6avbQUG8v6cF7FYyle1a1RWyAm1DE8MDq9Jw2SOoqJ/\n\
wE4gL0qpBDZ/KLbloJ+DO60S3z8CgYEA4uY7k6DMzGjovF4Ic8PhMItiMTMKlcpf\n\
1EYJKuybwbdfxrzjtUxvnFfcRFmgmhUnoatb2PPXKj3qoPpRFWAW8bvYWKQ24DE7\n\
fBwvlg9zWwFpZrO1RMsDBbSl41GoxxGCZhEILV40WntPy2LHhf7DQukMOhsF7dua\n\
/D2Khp67naECgYEAzbQLmf+6hHgWhp58Xy8zunGjo32GyxYh+OA0Pfh4xlogMDeO\n\
FONUR8Q6Ah7h+U9GcL5354yrJ5a6cVUXffaFGP19xZYgtFHGc1vcgrmlsZgTsYkT\n\
pcg6i4EIKtLy7BUPJhTVYR8TbeRmCYq8/FDF0YUDVeh+jpmPkF/qpBMH/48CgYBz\n\
UGU42wEaZbraeMO86fEZdc0aigE4LVjUjh98pDFomyRe4YKskkMq5vA4AIEBrfyt\n\
SmRsd0iD3GHRHEZ3IZWnlzsVmaeV+w9rPPvmPMX4m1gQ7QYUB0Tq8mtYgxjOyxRF\n\
gSRxwi3DSmY8TGBwthBQghZHtZIm13QF+9TaI/Pf4QKBgQCnWZhH4Dw9g9FMdis8\n\
fHc51gETWS1S0mSOY0qbtgjQgWJbVIfXCpwDNY7QMBhCYNdRTnG3NTCK996lqamN\n\
DsF8sk+TG9USSbyErC3WZJkWPm8IKpU+JRLe9SocSNhQ24KG7IOfY0XxuzsGLP9e\n\
AZpyPVEz1879HpIhxTgu/ZJfAQ==\n\
-----END PRIVATE KEY-----\n";

    pub(crate) fn credentials_json() -> String {
        json!({
            "type": "service_account",
            "client_email": "vertex-test@example-project.iam.gserviceaccount.com",
            "private_key": TEST_PRIVATE_KEY_PEM,
            "project_id": "example-project",
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::test_credentials::credentials_json;
    use super::*;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn decode_segment(segment: &str) -> Value {
        let bytes = URL_SAFE_NO_PAD.decode(segment).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /**
     * Minimal one-connection-per-response token endpoint; returns the served
     * URL, a request counter, and a capture of the last form body.
     */
    async fn spawn_token_server(
        responses: Vec<(u16, String)>,
    ) -> (String, Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let server_count = count.clone();
        let server_bodies = bodies.clone();
        tokio::spawn(async move {
            for (status, body) in responses {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                server_count.fetch_add(1, Ordering::SeqCst);
                let mut buf = vec![0_u8; 65536];
                let n = stream.read(&mut buf).await.unwrap();
                let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                let form = request
                    .split("\r\n\r\n")
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                server_bodies.lock().unwrap().push(form);
                let reason = if status == 200 { "OK" } else { "Error" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (format!("http://{addr}/token"), count, bodies)
    }

    fn token_body(token: &str, expires_in: u64) -> String {
        json!({ "access_token": token, "expires_in": expires_in, "token_type": "Bearer" })
            .to_string()
    }

    #[tokio::test]
    async fn mints_token_with_signed_jwt_bearer_assertion() {
        let (url, _, bodies) = spawn_token_server(vec![(200, token_body("tok_1", 3600))]).await;
        let source = ServiceAccountTokenSource::new(Some(credentials_json()), None, Some(url));

        let token = source.access_token().await.unwrap();

        assert_eq!(token, "tok_1");
        let form = bodies.lock().unwrap().last().cloned().unwrap();
        let assertion = form
            .split('&')
            .find_map(|pair| pair.strip_prefix("assertion="))
            .map(urlencoded_decode)
            .unwrap();
        assert!(form.contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer"));
        let segments: Vec<&str> = assertion.split('.').collect();
        assert_eq!(segments.len(), 3);
        assert_eq!(
            decode_segment(segments[0]),
            json!({ "alg": "RS256", "typ": "JWT" })
        );
        let claims = decode_segment(segments[1]);
        assert_eq!(
            claims["iss"],
            "vertex-test@example-project.iam.gserviceaccount.com"
        );
        assert_eq!(claims["scope"], CLOUD_PLATFORM_SCOPE);
        assert!(
            claims["aud"]
                .as_str()
                .unwrap()
                .starts_with("http://127.0.0.1:")
        );
        let iat = claims["iat"].as_u64().unwrap();
        let exp = claims["exp"].as_u64().unwrap();
        assert_eq!(exp - iat, TOKEN_LIFETIME.as_secs());
        assert!(!URL_SAFE_NO_PAD.decode(segments[2]).unwrap().is_empty());
    }

    #[tokio::test]
    async fn caches_token_until_expiry_slack() {
        let (url, count, _) = spawn_token_server(vec![
            (200, token_body("tok_1", 3600)),
            (200, token_body("tok_2", 3600)),
        ])
        .await;
        let source = ServiceAccountTokenSource::new(Some(credentials_json()), None, Some(url));

        assert_eq!(source.access_token().await.unwrap(), "tok_1");
        assert_eq!(source.access_token().await.unwrap(), "tok_1");
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn refreshes_token_inside_expiry_slack() {
        let (url, count, _) = spawn_token_server(vec![
            // Expires within the 60s slack, so the next call re-mints.
            (200, token_body("tok_1", 30)),
            (200, token_body("tok_2", 3600)),
        ])
        .await;
        let source = ServiceAccountTokenSource::new(Some(credentials_json()), None, Some(url));

        assert_eq!(source.access_token().await.unwrap(), "tok_1");
        assert_eq!(source.access_token().await.unwrap(), "tok_2");
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn exchange_failure_names_oauth_code_without_echoing_body() {
        let (url, _, _) = spawn_token_server(vec![(
            400,
            json!({ "error": "invalid_grant", "error_description": "Invalid JWT signature: secret-hint" })
                .to_string(),
        )])
        .await;
        let source = ServiceAccountTokenSource::new(Some(credentials_json()), None, Some(url));

        let err = source.access_token().await.unwrap_err().to_string();

        assert_eq!(
            err,
            "Vertex AI token exchange failed with 400 (invalid_grant); check the host clock and \
             that the service-account key is not revoked or deleted"
        );
        assert!(!err.contains("secret-hint"));
    }

    #[tokio::test]
    async fn missing_credentials_error_names_both_env_vars() {
        let source = ServiceAccountTokenSource::new(None, None, None);

        let err = source.access_token().await.unwrap_err().to_string();

        assert!(err.contains(CREDENTIALS_PATH_ENV), "{err}");
        assert!(err.contains(CREDENTIALS_JSON_ENV), "{err}");
    }

    #[tokio::test]
    async fn rejects_non_service_account_credentials() {
        let inline = json!({
            "type": "authorized_user",
            "client_email": "user@example.com",
            "private_key": "irrelevant",
        })
        .to_string();
        let source = ServiceAccountTokenSource::new(Some(inline), None, None);

        let err = source.access_token().await.unwrap_err().to_string();

        assert!(err.contains("service_account"), "{err}");
    }

    #[tokio::test]
    async fn rejects_malformed_credentials_json_without_echoing_it() {
        let source = ServiceAccountTokenSource::new(Some("{not json".to_string()), None, None);

        let err = source.access_token().await.unwrap_err().to_string();

        assert_eq!(
            err,
            format!("{CREDENTIALS_JSON_ENV} is not valid service-account JSON")
        );
    }

    #[test]
    fn debug_redacts_private_key() {
        let credentials = ServiceAccountCredentials::resolve(Some(&credentials_json()), None)
            .unwrap()
            .unwrap();

        let debug = format!("{credentials:?}");

        assert!(debug.contains("<redacted>"), "{debug}");
        assert!(!debug.contains("PRIVATE KEY"), "{debug}");
    }

    #[test]
    fn looks_configured_reflects_credential_presence() {
        assert!(
            ServiceAccountTokenSource::new(Some(credentials_json()), None, None).looks_configured()
        );
        assert!(
            !ServiceAccountTokenSource::new(None, Some("/nonexistent/key.json".to_string()), None)
                .looks_configured()
        );
        assert!(!ServiceAccountTokenSource::new(None, None, None).looks_configured());
    }

    #[test]
    fn resolves_project_id_from_credentials() {
        let source = ServiceAccountTokenSource::new(Some(credentials_json()), None, None);
        assert_eq!(
            source.project_from_credentials().as_deref(),
            Some("example-project")
        );
    }

    fn urlencoded_decode(value: &str) -> String {
        let mut out = Vec::new();
        let bytes = value.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'%' if i + 2 < bytes.len() => {
                    let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
                    out.push(u8::from_str_radix(hex, 16).unwrap());
                    i += 3;
                }
                b'+' => {
                    out.push(b' ');
                    i += 1;
                }
                byte => {
                    out.push(byte);
                    i += 1;
                }
            }
        }
        String::from_utf8(out).unwrap()
    }
}
