//! 使用者模型與查詢：Argon2 密碼雜湊/驗證、依 username/id 查詢、建立使用者。
//! User 為對外型別（不含密碼）；UserRow 含 password_hash 供登入驗證。

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub email: Option<String>,
    pub is_staff: bool,
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

pub async fn get_by_username(pool: &SqlitePool, username: &str) -> Result<Option<UserRow>, sqlx::Error> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, password_hash, email, is_staff FROM users WHERE username = ?",
    )
    .bind(username)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn get_by_id(pool: &SqlitePool, id: i64) -> Result<Option<User>, sqlx::Error> {
    let row = sqlx::query_as::<_, UserRow>("SELECT id, username, password_hash, email, is_staff FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.into()))
}

/// Internal row with password_hash; use get_by_id for public User.
#[derive(Debug)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub email: Option<String>,
    pub is_staff: i64,
}

impl From<UserRow> for User {
    fn from(r: UserRow) -> Self {
        User {
            id: r.id,
            username: r.username,
            email: r.email,
            is_staff: r.is_staff != 0,
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
        })
    }
}

/// 建立使用者（如種子或管理用）；回傳新使用者的 id。
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
    .bind(if is_staff { 1i64 } else { 0 })
    .execute(pool)
    .await?;
    Ok(res.last_insert_rowid())
}
