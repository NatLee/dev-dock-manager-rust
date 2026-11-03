//! 帳號 API。
//!
//! - 每個人：`GET /me` 看自己的帳號，`POST /me/password` 改自己的密碼。
//! - 管理員：`/users` 列出、新增使用者，`PATCH /users/:id` 改角色、停用或 email，
//!   `POST /users/:id/password` 重設別人的密碼。
//!
//! 管理員不能拿掉自己的管理員角色或停用自己，避免把最後一個管理員鎖在外面。

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, patch, post},
    Json, Router,
};
use serde::Deserialize;

use crate::auth_extractor::{AdminUser, AuthUser};
use crate::db::user::{self, check_new_password, User, UserChanges};
use crate::AppState;

type ApiError = (StatusCode, Json<serde_json::Value>);

fn api_error(status: StatusCode, message: impl ToString) -> ApiError {
    (status, Json(serde_json::json!({ "error": message.to_string() })))
}

fn db_error(e: sqlx::Error) -> ApiError {
    tracing::warn!("users api: {}", e);
    api_error(StatusCode::INTERNAL_SERVER_ERROR, "database error")
}

fn hash(password: &str) -> Result<String, ApiError> {
    check_new_password(password).map_err(|m| api_error(StatusCode::BAD_REQUEST, m))?;
    user::hash_password(password).map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "could not hash the password"))
}

async fn me(AuthUser(user): AuthUser) -> Json<User> {
    Json(user)
}

#[derive(Deserialize)]
struct ChangeOwnPassword {
    current_password: String,
    new_password: String,
}

async fn change_own_password(
    AuthUser(me): AuthUser,
    State(state): State<AppState>,
    Json(body): Json<ChangeOwnPassword>,
) -> Result<StatusCode, ApiError> {
    let row = user::get_row_by_id(&state.pool, me.id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "user not found"))?;
    if !user::verify_password(&row.password_hash, &body.current_password) {
        return Err(api_error(StatusCode::BAD_REQUEST, "The current password is wrong."));
    }
    let new_hash = hash(&body.new_password)?;
    user::set_password_hash(&state.pool, me.id, &new_hash).await.map_err(db_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_users(_admin: AdminUser, State(state): State<AppState>) -> Result<Json<Vec<User>>, ApiError> {
    Ok(Json(user::list_users(&state.pool).await.map_err(db_error)?))
}

#[derive(Deserialize)]
struct NewUser {
    username: String,
    password: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    is_admin: bool,
}

async fn create_user(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(body): Json<NewUser>,
) -> Result<(StatusCode, Json<User>), ApiError> {
    let username = body.username.trim();
    if username.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "Username is required."));
    }
    let password_hash = hash(&body.password)?;
    let email = body.email.as_deref().map(str::trim).filter(|e| !e.is_empty());
    let id = match user::create_user(&state.pool, username, &password_hash, email, body.is_admin).await {
        Ok(id) => id,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(api_error(StatusCode::BAD_REQUEST, "That username is already taken."));
        }
        Err(e) => return Err(db_error(e)),
    };
    let created = user::get_by_id(&state.pool, id).await.map_err(db_error)?.ok_or_else(|| db_error(sqlx::Error::RowNotFound))?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn update_user(
    AdminUser(admin): AdminUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(changes): Json<UserChanges>,
) -> Result<Json<User>, ApiError> {
    if id == admin.id && (changes.is_admin == Some(false) || changes.is_active == Some(false)) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "You can't remove your own admin role or disable your own account.",
        ));
    }
    if user::get_by_id(&state.pool, id).await.map_err(db_error)?.is_none() {
        return Err(api_error(StatusCode::NOT_FOUND, "User not found"));
    }
    user::update_user(&state.pool, id, &changes).await.map_err(db_error)?;
    let updated = user::get_by_id(&state.pool, id).await.map_err(db_error)?.ok_or_else(|| db_error(sqlx::Error::RowNotFound))?;
    Ok(Json(updated))
}

#[derive(Deserialize)]
struct ResetPassword {
    new_password: String,
}

async fn reset_password(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<ResetPassword>,
) -> Result<StatusCode, ApiError> {
    if user::get_by_id(&state.pool, id).await.map_err(db_error)?.is_none() {
        return Err(api_error(StatusCode::NOT_FOUND, "User not found"));
    }
    let new_hash = hash(&body.new_password)?;
    user::set_password_hash(&state.pool, id, &new_hash).await.map_err(db_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me", get(me))
        .route("/me/password", post(change_own_password))
        .route("/users", get(list_users).post(create_user))
        .route("/users/:id", patch(update_user))
        .route("/users/:id/password", post(reset_password))
}
