//! 認證：把 access token 換成目前的使用者，HTTP（Bearer header）和 WebSocket（第一則訊息）共用。
//!
//! 每次都重新讀資料庫，所以停用帳號或改角色會立刻生效，不用等 token 過期。

use async_trait::async_trait;
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};

use crate::db::{get_by_id, User};
use crate::jwt;
use crate::AppState;

/// 把 access token 換成仍然有效（存在且未停用）的使用者。
pub async fn authenticate(state: &AppState, token: &str) -> Result<User, (StatusCode, &'static str)> {
    let claims = jwt::verify_access(token, state.config.jwt_secret.as_bytes())
        .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid token"))?;
    let user = get_by_id(&state.pool, claims.user_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db error"))?
        .ok_or((StatusCode::UNAUTHORIZED, "user not found"))?;
    if !user.is_active {
        return Err((StatusCode::UNAUTHORIZED, "account disabled"));
    }
    Ok(user)
}

/// 已通過 JWT 驗證的使用者；若缺少或無效的 Bearer token 則回傳 401。
#[derive(Clone)]
pub struct AuthUser(pub User);

#[async_trait]
impl FromRequestParts<AppState> for AuthUser {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let auth = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .ok_or((StatusCode::UNAUTHORIZED, "missing Authorization"))?
            .to_str()
            .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid Authorization"))?;
        let token = auth
            .strip_prefix("Bearer ")
            .or_else(|| auth.strip_prefix("bearer "))
            .ok_or((StatusCode::UNAUTHORIZED, "missing Bearer"))?;
        authenticate(state, token).await.map(AuthUser)
    }
}

/// 管理員才能用的路由；一般使用者得到 403。
#[derive(Clone)]
pub struct AdminUser(pub User);

#[async_trait]
impl FromRequestParts<AppState> for AdminUser {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let AuthUser(user) = AuthUser::from_request_parts(parts, state).await?;
        if !user.is_staff {
            return Err((StatusCode::FORBIDDEN, "admins only"));
        }
        Ok(AdminUser(user))
    }
}
