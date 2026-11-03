//! JWT 認證 API：取得 token、refresh、驗證 token。
//! 登入以 username/password 換取 access/refresh token；無 Google 登入。

use axum::{
    extract::State,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::db::{get_by_username, verify_password};
use crate::AppState;

#[derive(Deserialize)]
pub struct TokenRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
}

#[derive(Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Serialize)]
pub struct RefreshResponse {
    pub access_token: String,
}

#[derive(Deserialize)]
pub struct VerifyRequest {
    pub token: Option<String>,
}

async fn token(
    State(state): State<AppState>,
    Json(body): Json<TokenRequest>,
) -> Result<Json<TokenResponse>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    let row = get_by_username(&state.pool, &body.username)
        .await
        .map_err(|e| {
            tracing::warn!("auth/token get_by_username error: {}", e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "detail": "database error" })),
            )
        })?;
    let row = row.ok_or_else(|| {
        tracing::info!("auth/token: user not found username={:?}", body.username);
        (
            axum::http::StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "detail": "Invalid credentials." })),
        )
    })?;
    // 停用的帳號和密碼錯誤回一樣的訊息，不透露帳號狀態。
    if row.is_active == 0 || !verify_password(&row.password_hash, &body.password) {
        tracing::info!("auth/token: rejected username={:?} (wrong password or disabled)", body.username);
        return Err((
            axum::http::StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "detail": "Invalid credentials." })),
        ));
    }
    let (access, refresh) = crate::jwt::issue_tokens(
        row.id,
        &row.username,
        state.config.jwt_secret.as_bytes(),
        3600,  // access 1h
        86400, // refresh 1d
    )
    .map_err(|_| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "detail": "token issue failed" })),
        )
    })?;
    Ok(Json(TokenResponse {
        access_token: access,
        refresh_token: refresh,
    }))
}

async fn refresh(
    State(state): State<AppState>,
    Json(body): Json<RefreshRequest>,
) -> Result<Json<RefreshResponse>, (axum::http::StatusCode, &'static str)> {
    let claims = crate::jwt::verify_refresh(&body.refresh_token, state.config.jwt_secret.as_bytes())
        .map_err(|_| (axum::http::StatusCode::UNAUTHORIZED, "invalid refresh token"))?;
    // 停用或刪除的帳號不能再換新的 access token。
    let still_active = crate::db::get_by_id(&state.pool, claims.user_id)
        .await
        .map_err(|_| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "db error"))?
        .is_some_and(|u| u.is_active);
    if !still_active {
        return Err((axum::http::StatusCode::UNAUTHORIZED, "invalid refresh token"));
    }
    let (access, _) = crate::jwt::issue_tokens(
        claims.user_id,
        &claims.sub,
        state.config.jwt_secret.as_bytes(),
        3600,
        86400,
    )
    .map_err(|_| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "token issue failed"))?;
    Ok(Json(RefreshResponse {
        access_token: access,
    }))
}

async fn verify(
    State(state): State<AppState>,
    Json(body): Json<VerifyRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    let token = body.token.ok_or_else(|| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "detail": "token required" })),
        )
    })?;
    // 和其他 API 同一套檢查：token 有效，而且帳號還在、沒被停用。
    crate::auth_extractor::authenticate(&state, &token).await.map_err(|_| {
        (
            axum::http::StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "detail": "Token is invalid or expired" })),
        )
    })?;
    Ok(Json(serde_json::json!({})))
}

/// 掛載 /auth/token（登入）、/auth/token/refresh、/auth/token/verify。
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/token", post(token))
        .route("/auth/token/refresh", post(refresh))
        .route("/auth/token/verify", post(verify))
}
