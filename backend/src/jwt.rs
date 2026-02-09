//! JWT 發放與驗證：access/refresh token；不處理 Google ID token。

use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct AccessClaims {
    pub sub: String,
    pub user_id: i64,
    pub exp: i64,
    pub iat: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RefreshClaims {
    pub sub: String,
    pub user_id: i64,
    pub exp: i64,
    pub iat: i64,
}

/// Issue access + refresh tokens (compatible with SimpleJWT-style response).
pub fn issue_tokens(
    user_id: i64,
    username: &str,
    secret: &[u8],
    access_ttl_secs: i64,
    refresh_ttl_secs: i64,
) -> Result<(String, String), jsonwebtoken::errors::Error> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let access = encode(
        &Header::default(),
        &AccessClaims {
            sub: username.to_string(),
            user_id,
            exp: now + access_ttl_secs,
            iat: now,
        },
        &EncodingKey::from_secret(secret),
    )?;
    let refresh = encode(
        &Header::default(),
        &RefreshClaims {
            sub: username.to_string(),
            user_id,
            exp: now + refresh_ttl_secs,
            iat: now,
        },
        &EncodingKey::from_secret(secret),
    )?;
    Ok((access, refresh))
}

/// Verify access token and return user_id.
pub fn verify_access(token: &str, secret: &[u8]) -> Result<AccessClaims, jsonwebtoken::errors::Error> {
    let d = decode::<AccessClaims>(
        token,
        &DecodingKey::from_secret(secret),
        &Validation::default(),
    )?;
    Ok(d.claims)
}

/// Verify refresh token.
pub fn verify_refresh(token: &str, secret: &[u8]) -> Result<RefreshClaims, jsonwebtoken::errors::Error> {
    let d = decode::<RefreshClaims>(
        token,
        &DecodingKey::from_secret(secret),
        &Validation::default(),
    )?;
    Ok(d.claims)
}
