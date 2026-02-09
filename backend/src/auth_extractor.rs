//! Axum extractor for JWT-authenticated user (Bearer token).
//!
//! 從請求的 `Authorization: Bearer <token>` 解析 JWT，驗證後查詢 DB 取得使用者，
//! 供需要登入的路由使用（例如 `AuthUser` 作為 handler 參數即可取得當前使用者）。

use async_trait::async_trait;
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};

use crate::db::{get_by_id, User};
use crate::jwt;
use crate::AppState;

/// 已通過 JWT 驗證的使用者；若缺少或無效的 Bearer token 則回傳 401。
#[derive(Clone)]
pub struct AuthUser(pub User);

#[async_trait]
impl FromRequestParts<AppState> for AuthUser {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let app = state;
        let auth = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .ok_or((StatusCode::UNAUTHORIZED, "missing Authorization"))?;
        let auth = auth
            .to_str()
            .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid Authorization"))?;
        let token = auth
            .strip_prefix("Bearer ")
            .or_else(|| auth.strip_prefix("bearer "))
            .ok_or((StatusCode::UNAUTHORIZED, "missing Bearer"))?;
        let claims = jwt::verify_access(token, app.config.jwt_secret.as_bytes())
            .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid token"))?;
        let user = get_by_id(&app.pool, claims.user_id)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db error"))?
            .ok_or((StatusCode::UNAUTHORIZED, "user not found"))?;
        Ok(AuthUser(user))
    }
}
