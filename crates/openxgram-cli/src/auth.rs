//! Web GUI 사용자 인증 — 이메일 + 비밀번호 + JWT.
//!
//! 정본: docs/PRD-OpenXgram.md §4.8 (Web GUI Beta).
//!
//! 흐름:
//!   - register : email + password (+ alias) → users 행 생성 (argon2id hash 저장) + JWT 발급
//!   - login    : email + password → password_hash 검증 + JWT 발급
//!   - verify   : Authorization: Bearer <JWT> → 서명 검증 + jwt_tokens lookup → user_id 반환
//!   - logout   : jwt_tokens.revoked = 1 (전체 invalidate 는 별도 vault rotate)
//!
//! 절대 규칙:
//!   - silent fallback 금지: 검증 실패 시 401 명시 (None X)
//!   - password plaintext 로그·DB 저장 절대 금지
//!   - JWT secret 은 jwt_secret 테이블 (32 bytes OS RNG, hex). 1회 생성 후 영속.
//!   - mcp_tokens Bearer 호환: daemon_gui::require_auth 는 JWT 또는 mcp-token 둘 다 수용.

use anyhow::{anyhow, bail, Context, Result};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use openxgram_db::Db;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// JWT 만료 — 7 일 (초 단위).
pub const JWT_TTL_SECONDS: i64 = 7 * 24 * 60 * 60;

/// 최소 비밀번호 길이 (영문 + 숫자 12자 이상).
pub const MIN_PASSWORD_LEN: usize = 12;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    pub sub: String,         // user_id (예: "user:<uuid>")
    pub email: String,
    pub iat: i64,            // issued at (unix seconds)
    pub exp: i64,            // expires at (unix seconds)
    pub jti: String,         // token id — jwt_tokens.id 와 동일
}

#[derive(Debug, Clone)]
pub struct UserRow {
    pub id: String,
    pub email: String,
    pub alias: Option<String>,
    pub role: String,
    pub created_at: i64,
    pub last_login_at: Option<i64>,
}

/// JWT 발급 결과 — `(user_row, encoded_jwt)`.
#[derive(Debug)]
pub struct AuthIssued {
    pub user: UserRow,
    pub jwt: String,
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// jwt_secret 행 ensure — 없으면 OS RNG 32바이트 생성 후 hex 저장.
fn ensure_jwt_secret(db: &mut Db) -> Result<Vec<u8>> {
    let existing: Option<String> = db
        .conn()
        .query_row("SELECT secret_hex FROM jwt_secret WHERE id = 1", [], |r| {
            r.get(0)
        })
        .ok();
    if let Some(hex_s) = existing {
        return hex::decode(&hex_s).context("jwt_secret hex decode 실패");
    }
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow!("OS RNG 실패: {e}"))?;
    let hex_s = hex::encode(bytes);
    let now = now_unix();
    let affected = db.conn().execute(
        "INSERT INTO jwt_secret (id, secret_hex, created_at) VALUES (1, ?1, ?2)",
        rusqlite::params![hex_s, now],
    )?;
    if affected != 1 {
        bail!("jwt_secret 저장 실패 (affected={affected})");
    }
    Ok(bytes.to_vec())
}

/// argon2id 로 해싱 — 인코딩된 문자열 ("$argon2id$v=19$...") 반환.
pub fn hash_password(password: &str) -> Result<String> {
    if password.len() < MIN_PASSWORD_LEN {
        bail!("비밀번호 최소 {MIN_PASSWORD_LEN}자");
    }
    let mut salt_bytes = [0u8; 16];
    getrandom::fill(&mut salt_bytes).map_err(|e| anyhow!("OS RNG (salt) 실패: {e}"))?;
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|e| anyhow!("argon2 salt encode 실패: {e}"))?;
    let argon = Argon2::default();
    let hash = argon
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow!("argon2 hash 실패: {e}"))?;
    Ok(hash.to_string())
}

/// 저장된 argon2 인코딩 hash 와 평문 password 비교.
pub fn verify_password(password: &str, encoded: &str) -> Result<bool> {
    let parsed = PasswordHash::new(encoded).map_err(|e| anyhow!("argon2 parse 실패: {e}"))?;
    match Argon2::default().verify_password(password.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(e) => Err(anyhow!("argon2 verify 실패: {e}")),
    }
}

fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

/// JWT HS256 인코딩 — `base64url(header).base64url(payload).base64url(signature)`.
pub fn encode_jwt(claims: &JwtClaims, secret: &[u8]) -> Result<String> {
    let header = r#"{"alg":"HS256","typ":"JWT"}"#;
    let header_b64 = URL_SAFE_NO_PAD.encode(header.as_bytes());
    let payload_json = serde_json::to_string(claims).context("claims serialize 실패")?;
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload_json.as_bytes());
    let signing_input = format!("{header_b64}.{payload_b64}");

    let mut mac = HmacSha256::new_from_slice(secret).map_err(|e| anyhow!("hmac key 실패: {e}"))?;
    mac.update(signing_input.as_bytes());
    let sig = mac.finalize().into_bytes();
    let sig_b64 = URL_SAFE_NO_PAD.encode(sig);
    Ok(format!("{signing_input}.{sig_b64}"))
}

/// JWT 서명 검증 + claims 디코드. 만료(exp) 또한 검사.
pub fn decode_jwt(token: &str, secret: &[u8]) -> Result<JwtClaims> {
    let mut parts = token.split('.');
    let header = parts.next().ok_or_else(|| anyhow!("jwt 형식 오류"))?;
    let payload = parts.next().ok_or_else(|| anyhow!("jwt 형식 오류"))?;
    let sig = parts.next().ok_or_else(|| anyhow!("jwt 형식 오류"))?;
    if parts.next().is_some() {
        bail!("jwt 형식 오류 (점 3개 초과)");
    }

    let signing_input = format!("{header}.{payload}");
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|e| anyhow!("hmac key 실패: {e}"))?;
    mac.update(signing_input.as_bytes());
    let expected = mac.finalize().into_bytes();
    let actual = URL_SAFE_NO_PAD
        .decode(sig)
        .map_err(|e| anyhow!("jwt 서명 base64 디코드 실패: {e}"))?;
    if expected.as_slice() != actual.as_slice() {
        bail!("jwt 서명 불일치");
    }

    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|e| anyhow!("jwt payload base64 디코드 실패: {e}"))?;
    let claims: JwtClaims =
        serde_json::from_slice(&payload_bytes).context("jwt payload json 디코드 실패")?;

    if claims.exp < now_unix() {
        bail!("jwt 만료됨");
    }
    Ok(claims)
}

fn fetch_user_by_email(db: &mut Db, email: &str) -> Result<Option<(String, String, UserRow)>> {
    let res = db.conn().query_row(
        "SELECT id, email, password_hash, alias, role, created_at, last_login_at
         FROM users WHERE email = ?1",
        [email],
        |r| {
            Ok((
                r.get::<_, String>(0)?, // id
                r.get::<_, String>(1)?, // email
                r.get::<_, String>(2)?, // password_hash
                r.get::<_, Option<String>>(3)?, // alias
                r.get::<_, String>(4)?, // role
                r.get::<_, i64>(5)?,    // created_at
                r.get::<_, Option<i64>>(6)?, // last_login_at
            ))
        },
    );
    match res {
        Ok((id, email, hash, alias, role, created_at, last_login_at)) => {
            let user = UserRow {
                id: id.clone(),
                email,
                alias,
                role,
                created_at,
                last_login_at,
            };
            Ok(Some((id, hash, user)))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn fetch_user_by_id(db: &mut Db, user_id: &str) -> Result<Option<UserRow>> {
    let res = db.conn().query_row(
        "SELECT id, email, alias, role, created_at, last_login_at
         FROM users WHERE id = ?1",
        [user_id],
        |r| {
            Ok(UserRow {
                id: r.get(0)?,
                email: r.get(1)?,
                alias: r.get(2)?,
                role: r.get(3)?,
                created_at: r.get(4)?,
                last_login_at: r.get(5)?,
            })
        },
    );
    match res {
        Ok(u) => Ok(Some(u)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// 등록된 사용자가 1명도 없는지 — 첫 사용자는 admin 자동 승격.
fn is_first_user(db: &mut Db) -> Result<bool> {
    let n: i64 =
        db.conn()
            .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
    Ok(n == 0)
}

fn issue_token(db: &mut Db, user: &UserRow, secret: &[u8]) -> Result<String> {
    let jti = Uuid::new_v4().to_string();
    let iat = now_unix();
    let exp = iat + JWT_TTL_SECONDS;
    let claims = JwtClaims {
        sub: user.id.clone(),
        email: user.email.clone(),
        iat,
        exp,
        jti: jti.clone(),
    };
    let jwt = encode_jwt(&claims, secret)?;
    let token_hash = sha256_hex(&jwt);
    let affected = db.conn().execute(
        "INSERT INTO jwt_tokens (id, user_id, token_hash, issued_at, expires_at, revoked)
         VALUES (?1, ?2, ?3, ?4, ?5, 0)",
        rusqlite::params![jti, user.id, token_hash, iat, exp],
    )?;
    if affected != 1 {
        bail!("jwt_tokens insert 실패 (affected={affected})");
    }
    Ok(jwt)
}

/// 사용자 등록 — 이메일 중복 시 conflict.
pub fn register(
    db: &mut Db,
    email: &str,
    password: &str,
    alias: Option<&str>,
) -> Result<AuthIssued> {
    let email = email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        bail!("이메일 형식 오류");
    }
    if password.len() < MIN_PASSWORD_LEN {
        bail!("비밀번호 최소 {MIN_PASSWORD_LEN}자");
    }
    if fetch_user_by_email(db, &email)?.is_some() {
        bail!("이미 가입된 이메일");
    }

    let first = is_first_user(db)?;
    let role = if first { "admin" } else { "user" };
    let id = format!("user:{}", Uuid::new_v4());
    let pw_hash = hash_password(password)?;
    let now = now_unix();
    let affected = db.conn().execute(
        "INSERT INTO users (id, email, password_hash, alias, role, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        rusqlite::params![id, email, pw_hash, alias, role, now],
    )?;
    if affected != 1 {
        bail!("users insert 실패 (affected={affected})");
    }
    let user = UserRow {
        id: id.clone(),
        email: email.clone(),
        alias: alias.map(|s| s.to_string()),
        role: role.to_string(),
        created_at: now,
        last_login_at: None,
    };
    let secret = ensure_jwt_secret(db)?;
    let jwt = issue_token(db, &user, &secret)?;
    Ok(AuthIssued { user, jwt })
}

/// 로그인 — email + password 검증.
pub fn login(db: &mut Db, email: &str, password: &str) -> Result<AuthIssued> {
    let email = email.trim().to_lowercase();
    let (id, hash, mut user) = match fetch_user_by_email(db, &email)? {
        Some(t) => t,
        None => bail!("이메일/비밀번호 불일치"),
    };
    if !verify_password(password, &hash)? {
        bail!("이메일/비밀번호 불일치");
    }
    let now = now_unix();
    db.conn().execute(
        "UPDATE users SET last_login_at = ?1, updated_at = ?1 WHERE id = ?2",
        rusqlite::params![now, id],
    )?;
    user.last_login_at = Some(now);
    let secret = ensure_jwt_secret(db)?;
    let jwt = issue_token(db, &user, &secret)?;
    Ok(AuthIssued { user, jwt })
}

/// JWT 검증 — 서명·만료·revoked 모두 검사. user_id 반환.
pub fn verify_jwt(db: &mut Db, jwt: &str) -> Result<Option<UserRow>> {
    let secret = ensure_jwt_secret(db)?;
    let claims = match decode_jwt(jwt, &secret) {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    let token_hash = sha256_hex(jwt);
    let row: Option<i64> = db
        .conn()
        .query_row(
            "SELECT revoked FROM jwt_tokens WHERE id = ?1 AND token_hash = ?2",
            rusqlite::params![claims.jti, token_hash],
            |r| r.get(0),
        )
        .ok();
    match row {
        Some(0) => fetch_user_by_id(db, &claims.sub),
        Some(_) => Ok(None), // revoked
        None => Ok(None),    // 미존재 (DB 리셋 / 위조)
    }
}

/// 로그아웃 — 지정 JWT revoke (다른 디바이스는 영향 X).
pub fn logout(db: &mut Db, jwt: &str) -> Result<()> {
    let secret = ensure_jwt_secret(db)?;
    let claims = decode_jwt(jwt, &secret).context("jwt decode 실패")?;
    let token_hash = sha256_hex(jwt);
    db.conn().execute(
        "UPDATE jwt_tokens SET revoked = 1 WHERE id = ?1 AND token_hash = ?2",
        rusqlite::params![claims.jti, token_hash],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use openxgram_db::DbConfig;

    fn temp_db() -> Db {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut db = Db::open(DbConfig {
            path,
            ..Default::default()
        })
        .unwrap();
        db.migrate().unwrap();
        // tempdir 가 drop 되어도 path 가 살아있도록 keep
        std::mem::forget(dir);
        db
    }

    #[test]
    fn password_hash_roundtrip() {
        let h = hash_password("strong-password-123").unwrap();
        assert!(verify_password("strong-password-123", &h).unwrap());
        assert!(!verify_password("wrong-password", &h).unwrap());
    }

    #[test]
    fn password_too_short() {
        assert!(hash_password("short").is_err());
    }

    #[test]
    fn jwt_roundtrip() {
        let secret = b"0123456789abcdef0123456789abcdef";
        let claims = JwtClaims {
            sub: "user:abc".into(),
            email: "x@y.com".into(),
            iat: 1000,
            exp: now_unix() + 3600,
            jti: "jti-1".into(),
        };
        let jwt = encode_jwt(&claims, secret).unwrap();
        let decoded = decode_jwt(&jwt, secret).unwrap();
        assert_eq!(decoded.sub, "user:abc");
        assert_eq!(decoded.jti, "jti-1");
    }

    #[test]
    fn jwt_tampered_signature_rejected() {
        let secret = b"0123456789abcdef0123456789abcdef";
        let claims = JwtClaims {
            sub: "user:abc".into(),
            email: "x@y.com".into(),
            iat: 1000,
            exp: now_unix() + 3600,
            jti: "jti-1".into(),
        };
        let jwt = encode_jwt(&claims, secret).unwrap();
        let tampered = format!("{jwt}A");
        assert!(decode_jwt(&tampered, secret).is_err());
    }

    #[test]
    fn register_first_user_is_admin() {
        let mut db = temp_db();
        let r = register(&mut db, "a@b.com", "password-1234", Some("alpha")).unwrap();
        assert_eq!(r.user.role, "admin");
        let r2 = register(&mut db, "c@d.com", "password-1234", None).unwrap();
        assert_eq!(r2.user.role, "user");
    }

    #[test]
    fn register_then_login_then_verify() {
        let mut db = temp_db();
        let r = register(&mut db, "a@b.com", "password-1234", None).unwrap();
        let l = login(&mut db, "a@b.com", "password-1234").unwrap();
        assert_eq!(l.user.id, r.user.id);

        let verified = verify_jwt(&mut db, &l.jwt).unwrap().unwrap();
        assert_eq!(verified.id, r.user.id);
    }

    #[test]
    fn login_wrong_password_fails() {
        let mut db = temp_db();
        register(&mut db, "a@b.com", "password-1234", None).unwrap();
        assert!(login(&mut db, "a@b.com", "WRONG-password").is_err());
    }

    #[test]
    fn logout_revokes_token() {
        let mut db = temp_db();
        let r = register(&mut db, "a@b.com", "password-1234", None).unwrap();
        assert!(verify_jwt(&mut db, &r.jwt).unwrap().is_some());
        logout(&mut db, &r.jwt).unwrap();
        assert!(verify_jwt(&mut db, &r.jwt).unwrap().is_none());
    }

    #[test]
    fn duplicate_email_rejected() {
        let mut db = temp_db();
        register(&mut db, "a@b.com", "password-1234", None).unwrap();
        assert!(register(&mut db, "a@b.com", "password-1234", None).is_err());
    }
}
