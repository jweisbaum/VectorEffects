//! Signature V4 for read-only S3 requests. Credentials never enter URLs or logs.

use hmac::{Hmac, Mac};
use reqwest::blocking::RequestBuilder;
use reqwest::header::{AUTHORIZATION, HeaderValue};
use reqwest::{Method, Url};
use sha2::{Digest, Sha256};
use zarrs_storage::StorageError;

#[derive(Clone)]
pub(crate) struct Credentials {
    pub access_key: &'static str,
    pub secret_key: &'static str,
    pub region: &'static str,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("region", &self.region)
            .finish_non_exhaustive()
    }
}

fn mac(key: &[u8], message: &str) -> Result<Vec<u8>, StorageError> {
    let mut hmac = Hmac::<Sha256>::new_from_slice(key)
        .map_err(|_| StorageError::Other("could not initialize S3 signing".into()))?;
    hmac.update(message.as_bytes());
    Ok(hmac.finalize().into_bytes().to_vec())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub(crate) fn canonical_query(url: &Url) -> String {
    let mut pairs: Vec<_> = url
        .query_pairs()
        .map(|(k, v)| (encode(&k), encode(&v)))
        .collect();
    pairs.sort();
    pairs
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

impl Credentials {
    fn authorization(
        &self,
        method: &Method,
        url: &Url,
        date: &str,
        payload: &str,
    ) -> Result<String, StorageError> {
        // Object keys are fixed ASCII paths. Listings add escaped and sorted
        // query parameters, including opaque continuation tokens.
        if url.scheme() != "https" || url.port().is_some() {
            return Err(StorageError::Other("unsupported S3 request URL".into()));
        }
        let host = url
            .host_str()
            .ok_or_else(|| StorageError::Other("S3 URL has no host".into()))?;
        let headers = "host;x-amz-content-sha256;x-amz-date";
        let canonical = format!(
            "{method}\n{}\n{}\nhost:{host}\nx-amz-content-sha256:{payload}\nx-amz-date:{date}\n\n{headers}\n{payload}",
            url.path(),
            canonical_query(url)
        );
        let day = &date[..8];
        let scope = format!("{day}/{}/s3/aws4_request", self.region);
        let to_sign = format!(
            "AWS4-HMAC-SHA256\n{date}\n{scope}\n{}",
            hex(&Sha256::digest(canonical.as_bytes()))
        );
        let mut key = mac(format!("AWS4{}", self.secret_key).as_bytes(), day)?;
        for part in [self.region, "s3", "aws4_request"] {
            key = mac(&key, part)?;
        }
        Ok(format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={headers}, Signature={}",
            self.access_key,
            hex(&mac(&key, &to_sign)?)
        ))
    }

    pub fn sign(
        &self,
        builder: RequestBuilder,
        method: &Method,
        url: &Url,
    ) -> Result<RequestBuilder, StorageError> {
        let date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let payload = hex(&Sha256::digest([]));
        let mut authorization =
            HeaderValue::from_str(&self.authorization(method, url, &date, &payload)?)
                .map_err(|_| StorageError::Other("invalid S3 authorization".into()))?;
        authorization.set_sensitive(true);
        Ok(builder
            .header("x-amz-date", date)
            .header("x-amz-content-sha256", payload)
            .header(AUTHORIZATION, authorization))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn signatures_cover_the_method_path_host_and_date_without_exposing_secrets() {
        let credentials = Credentials {
            access_key: "test-access",
            secret_key: "test-secret",
            region: "us-east-1",
        };
        let url = Url::parse("https://store.invalid/test.txt").unwrap();
        let payload = hex(&Sha256::digest([]));
        // Fixed independent Python hashlib/hmac reference calculations.
        for (method, expected) in [
            (
                Method::GET,
                "40320c63f42bf493a3215a1f91c8de01875dc1a702ecbc79f3513f292cddb25c",
            ),
            (
                Method::HEAD,
                "2dee360956e9dcac9452a195d2321b78abb1c24bb12cb34322895420f2950059",
            ),
        ] {
            let auth = credentials
                .authorization(&method, &url, "20130524T000000Z", &payload)
                .unwrap();
            assert!(auth.ends_with(expected));
            assert!(auth.contains("Credential=test-access/20130524/us-east-1/s3/aws4_request"));
        }
        assert!(!format!("{credentials:?}").contains("test-secret"));
        let client = reqwest::blocking::Client::new();
        let request = credentials
            .sign(client.get(url.clone()), &Method::GET, &url)
            .unwrap()
            .build()
            .unwrap();
        assert!(request.headers()[AUTHORIZATION].is_sensitive());
        assert!(!format!("{request:?}").contains("test-access"));
    }
}
