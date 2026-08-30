use axum::http::{header, HeaderMap};

use crate::error::ApiError;

/*
 * Autenticacion por bearer token compartido. La comparacion es
 *
 * V1: un unico token compartido, configurado en el Collector via env
 * `OBS_COLLECTOR_TOKEN`, que cada agent envia en `Authorization: Bearer
 * <token>`. Comparacion en tiempo (casi) constante para no filtrar
 * longitud/informacion por timing.
 */

pub fn check_bearer(headers: &HeaderMap, expected: &str) -> Result<(), ApiError> {
    let header_value = headers
        .get(header::AUTHORIZATION)
        .ok_or_else(ApiError::unauthorized)?;

    let value = header_value
        .to_str()
        .map_err(|_| ApiError::unauthorized())?;

    let token = value
        .strip_prefix("Bearer ")
        .ok_or_else(ApiError::unauthorized)?;

    check_bearer_str(token, expected)
}

/*
 * La misma comparacion pero sobre un token ya extraido: la usa el
 * WebSocket (6.2), donde el cliente puede pasar el token por query param
 * (el API de WebSocket de un navegador no deja setear headers).
 */
pub fn check_bearer_str(token: &str, expected: &str) -> Result<(), ApiError> {
    if constant_time_eq(token.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err(ApiError::unauthorized())
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
    use axum::http::HeaderMap;

    fn headers_with(values: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in values {
            let key = axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap();
            let value = v.parse().unwrap();
            h.insert(key, value);
        }
        h
    }

    #[test]
    fn accepts_correct_bearer() {
        let h = headers_with(&[(AUTHORIZATION.as_str(), "Bearer secret-token")]);
        assert!(check_bearer(&h, "secret-token").is_ok());
    }

    #[test]
    fn rejects_missing_header() {
        let h = headers_with(&[(CONTENT_TYPE.as_str(), "application/json")]);
        assert!(check_bearer(&h, "secret-token").is_err());
    }

    #[test]
    fn rejects_wrong_scheme() {
        let h = headers_with(&[(AUTHORIZATION.as_str(), "Basic abc")]);
        assert!(check_bearer(&h, "secret-token").is_err());
    }

    #[test]
    fn rejects_wrong_token() {
        let h = headers_with(&[(AUTHORIZATION.as_str(), "Bearer other")]);
        assert!(check_bearer(&h, "secret-token").is_err());
    }

    #[test]
    fn rejects_wrong_len_token() {
        let h = headers_with(&[(AUTHORIZATION.as_str(), "Bearer short")]);
        assert!(check_bearer(&h, "secret-token").is_err());
    }

    #[test]
    fn auth_str_accepts_correct_token() {
        assert!(check_bearer_str("secret-token", "secret-token").is_ok());
    }

    #[test]
    fn auth_str_rejects_wrong_token() {
        assert!(check_bearer_str("other", "secret-token").is_err());
        assert!(check_bearer_str("", "secret-token").is_err());
    }

    #[test]
    fn constant_time_matches_differing_prefix() {
        assert!(!constant_time_eq(b"abc", b"xyz"));
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abcd", b"xyz"));
    }
}
