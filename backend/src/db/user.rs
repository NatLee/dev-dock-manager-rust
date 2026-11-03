//! 使用者模型與查詢：Argon2 密碼雜湊/驗證、依 username/id 查詢、建立與管理使用者。
//! User 為對外型別（不含密碼）；UserRow 含 password_hash 供登入驗證。
//!
//! 角色只有兩種：管理員（`is_staff`）和一般使用者。停用的帳號（`is_active = 0`）不能登入，
//! 舊的 token 也會失效（每個請求都會重新讀取使用者）。

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// 網頁上設定密碼時的最短長度（和 Django 預設的 MinimumLengthValidator 一樣）。
/// 命令列的 create-user 不檢查，方便第一次建立管理員。
pub const MIN_PASSWORD_LENGTH: usize = 8;

const USER_COLUMNS: &str = "id, username, password_hash, email, is_staff, is_active, created_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub email: Option<String>,
    /// 管理員：看得到、管得到所有容器，可以建立 privileged 容器、管理使用者。
    #[serde(rename = "is_admin")]
    pub is_staff: bool,
    pub is_active: bool,
    pub created_at: Option<String>,
}

impl User {
    /// 這個使用者能不能看到、操作屬於 `owner_id` 的容器（沒有主人的舊容器只有管理員能碰）。
    pub fn can_access(&self, owner_id: Option<i64>) -> bool {
        self.is_staff || owner_id == Some(self.id)
    }
}

/// Hash a password with Argon2 (for storage).
pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
}

/// Verify plain password against stored hash.
pub fn verify_password(hash: &str, password: &str) -> bool {
    let parsed = match PasswordHash::new(hash) {
        Ok(p) => p,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// 網頁設定密碼時的規則；回傳給使用者看的錯誤訊息。
pub fn check_new_password(password: &str) -> Result<(), String> {
    if password.chars().count() < MIN_PASSWORD_LENGTH {
        return Err(format!(
            "Password must be at least {} characters long.",
            MIN_PASSWORD_LENGTH
        ));
    }
    Ok(())
}

pub async fn get_by_username(pool: &SqlitePool, username: &str) -> Result<Option<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRow>(&format!("SELECT {USER_COLUMNS} FROM users WHERE username = ?"))
        .bind(username)
        .fetch_optional(pool)
        .await
}

pub async fn get_row_by_id(pool: &SqlitePool, id: i64) -> Result<Option<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRow>(&format!("SELECT {USER_COLUMNS} FROM users WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn get_by_id(pool: &SqlitePool, id: i64) -> Result<Option<User>, sqlx::Error> {
    Ok(get_row_by_id(pool, id).await?.map(Into::into))
}

pub async fn list_users(pool: &SqlitePool) -> Result<Vec<User>, sqlx::Error> {
    let rows = sqlx::query_as::<_, UserRow>(&format!("SELECT {USER_COLUMNS} FROM users ORDER BY id"))
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// Internal row with password_hash; use get_by_id for public User.
#[derive(Debug)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub email: Option<String>,
    pub is_staff: i64,
    pub is_active: i64,
    pub created_at: Option<String>,
}

impl From<UserRow> for User {
    fn from(r: UserRow) -> Self {
        User {
            id: r.id,
            username: r.username,
            email: r.email,
            is_staff: r.is_staff != 0,
            is_active: r.is_active != 0,
            created_at: r.created_at,
        }
    }
}

impl<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> for UserRow {
    fn from_row(row: &'r sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(UserRow {
            id: row.try_get("id")?,
            username: row.try_get("username")?,
            password_hash: row.try_get("password_hash")?,
            email: row.try_get("email")?,
            is_staff: row.try_get("is_staff")?,
            is_active: row.try_get("is_active")?,
            created_at: row.try_get("created_at")?,
        })
    }
}

/// 建立使用者（命令列或管理頁面）；回傳新使用者的 id。
pub async fn create_user(
    pool: &SqlitePool,
    username: &str,
    password_hash: &str,
    email: Option<&str>,
    is_staff: bool,
) -> Result<i64, sqlx::Error> {
    let res = sqlx::query(
        "INSERT INTO users (username, password_hash, email, is_staff) VALUES (?, ?, ?, ?)",
    )
    .bind(username)
    .bind(password_hash)
    .bind(email)
    .bind(is_staff)
    .execute(pool)
    .await?;
    Ok(res.last_insert_rowid())
}

/// 管理頁面能改的欄位；`None` 表示不改。
#[derive(Debug, Default, Deserialize)]
pub struct UserChanges {
    pub email: Option<String>,
    pub is_admin: Option<bool>,
    pub is_active: Option<bool>,
}

pub async fn update_user(pool: &SqlitePool, id: i64, changes: &UserChanges) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE users SET email = COALESCE(?, email), is_staff = COALESCE(?, is_staff), \
         is_active = COALESCE(?, is_active) WHERE id = ?",
    )
    .bind(changes.email.as_deref())
    .bind(changes.is_admin)
    .bind(changes.is_active)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_password_hash(pool: &SqlitePool, id: i64, password_hash: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(password_hash)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
