//! The bearer token a client must present (spec.md 8.8).

use base64::Engine;

/// Thirty-two random bytes, base64url without padding: 43 characters.
pub fn fresh() -> String {
    let mut bytes = [0u8; 32];
    // The OS random source failing is a broken machine; an empty token would
    // then match nothing, which is the safe failure.
    if getrandom::fill(&mut bytes).is_err() {
        return String::new();
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Whether an `Authorization` header value is `Bearer <token>` for this token.
///
/// An empty token matches nothing: the service is never open by accident.
pub fn matches(header: Option<&str>, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let Some(value) = header else { return false };
    let Some(presented) = value.strip_prefix("Bearer ") else {
        return false;
    };
    constant_time_eq(presented.trim().as_bytes(), token.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_token_is_43_url_safe_characters_and_unique() {
        let a = fresh();
        let b = fresh();
        assert_eq!(a.len(), 43);
        assert!(
            a.bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        );
        assert_ne!(a, b);
    }

    #[test]
    fn only_the_exact_bearer_matches() {
        let token = fresh();
        assert!(matches(Some(&format!("Bearer {token}")), &token));
        assert!(!matches(Some(&token), &token));
        assert!(!matches(Some("Bearer nope"), &token));
        assert!(!matches(None, &token));
        assert!(!matches(Some("Bearer "), ""));
    }
}
