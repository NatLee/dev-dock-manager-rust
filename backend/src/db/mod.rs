//! 資料庫層：使用者查詢與密碼驗證（僅 JWT 登入，無 SocialAccount/Google）。

pub mod user;

pub use user::User;
pub use user::{get_by_id, get_by_username, verify_password};
pub use user::create_user;
pub use user::hash_password;
