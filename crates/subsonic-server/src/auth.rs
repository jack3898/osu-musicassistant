//! Subsonic authentication: salted token (`u`, `t`, `s`), plain or hex-encoded password (`u`, `p`)
//! and the OpenSubsonic API key extension (`apiKey`).

use md5::{Digest, Md5};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub username: String,
    pub password: String,
    pub api_key: String,
}

/// Why a request was rejected, mapped to Subsonic error codes by the API layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    /// No usable credentials were sent (code 10).
    Missing,
    /// Credentials were sent but are wrong (code 40).
    Invalid,
}

impl Credentials {
    pub fn verify(&self, param: impl Fn(&str) -> Option<String>) -> Result<(), AuthError> {
        if let Some(key) = param("apiKey") {
            return check(constant_time_eq(key.as_bytes(), self.api_key.as_bytes()));
        }
        let user = param("u").ok_or(AuthError::Missing)?;
        let user_ok = constant_time_eq(user.as_bytes(), self.username.as_bytes());
        if let (Some(token), Some(salt)) = (param("t"), param("s")) {
            let expected = hex::encode(Md5::digest(format!("{}{salt}", self.password).as_bytes()));
            return check(
                user_ok & constant_time_eq(token.to_lowercase().as_bytes(), expected.as_bytes()),
            );
        }
        let password = param("p").ok_or(AuthError::Missing)?;
        let password = match password.strip_prefix("enc:") {
            Some(encoded) => hex::decode(encoded)
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .ok_or(AuthError::Invalid)?,
            None => password,
        };
        check(user_ok & constant_time_eq(password.as_bytes(), self.password.as_bytes()))
    }
}

fn check(ok: bool) -> Result<(), AuthError> {
    if ok { Ok(()) } else { Err(AuthError::Invalid) }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn creds() -> Credentials {
        Credentials {
            username: "osu".into(),
            password: "sesame".into(),
            api_key: "key123".into(),
        }
    }

    fn verify(params: &[(&str, &str)]) -> Result<(), AuthError> {
        let map: HashMap<_, _> = params
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        creds().verify(|k| map.get(k).cloned())
    }

    #[test]
    fn token_auth() {
        let token = hex::encode(Md5::digest(b"sesamec19b2d"));
        assert_eq!(
            verify(&[("u", "osu"), ("t", &token), ("s", "c19b2d")]),
            Ok(())
        );
        assert_eq!(
            verify(&[("u", "osu"), ("t", &token), ("s", "other")]),
            Err(AuthError::Invalid)
        );
        assert_eq!(
            verify(&[("u", "someone"), ("t", &token), ("s", "c19b2d")]),
            Err(AuthError::Invalid)
        );
    }

    #[test]
    fn password_and_api_key_auth() {
        assert_eq!(verify(&[("u", "osu"), ("p", "sesame")]), Ok(()));
        assert_eq!(verify(&[("u", "osu"), ("p", "enc:736573616d65")]), Ok(()));
        assert_eq!(
            verify(&[("u", "osu"), ("p", "wrong")]),
            Err(AuthError::Invalid)
        );
        assert_eq!(verify(&[("apiKey", "key123")]), Ok(()));
        assert_eq!(verify(&[("apiKey", "nope")]), Err(AuthError::Invalid));
        assert_eq!(verify(&[]), Err(AuthError::Missing));
    }
}
