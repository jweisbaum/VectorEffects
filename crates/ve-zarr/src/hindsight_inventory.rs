//! A fresh, paginated inventory of committed Hindsight time chunks.
//! The allocated time axis includes unwritten dates and is not coverage.

use super::*;
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Debug, Deserialize)]
#[serde(rename = "ListBucketResult", rename_all = "PascalCase")]
struct Page {
    is_truncated: bool,
    next_continuation_token: Option<String>,
    #[serde(default, rename = "Contents")]
    contents: Vec<Object>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Object {
    key: String,
    size: u64,
}

fn parse_page(xml: &[u8]) -> std::result::Result<Page, StorageError> {
    quick_xml::de::from_reader(xml)
        .map_err(|e| StorageError::Other(format!("invalid Hindsight completion listing: {e}")))
}

impl HttpStore {
    pub(crate) fn completed_chunks(&self) -> Result<BTreeSet<u64>> {
        // The configured store is one directory beneath its bucket, whether
        // the endpoint addresses that bucket by path or by virtual host.
        let (bucket_path, root) = self
            .base
            .path()
            .trim_end_matches('/')
            .rsplit_once('/')
            .ok_or_else(|| ZarrError::Open("Hindsight store has no bucket path".into()))?;
        let prefix = format!("{root}/.historysyncer/complete/");
        let mut url = self.base.clone();
        url.set_path(&format!("{bucket_path}/"));
        let mut token: Option<String> = None;
        let mut seen = BTreeSet::new();
        let mut complete = BTreeSet::new();
        loop {
            url.set_query(None);
            url.query_pairs_mut()
                .append_pair("list-type", "2")
                .append_pair("prefix", &prefix);
            if let Some(token) = &token {
                url.query_pairs_mut()
                    .append_pair("continuation-token", token);
            }
            let query = crate::s3::canonical_query(&url);
            url.set_query(Some(&query));
            let page = retrying(|| {
                let _permit = self.lane.permit();
                let response = self
                    .request(reqwest::Method::GET, &url)
                    .map_err(Retry::No)?
                    .send()
                    .map_err(|e| Retry::Later(failed(e)))?;
                if response.status() != StatusCode::OK {
                    let status = response.status();
                    let error = StorageError::Other(format!(
                        "could not list Hindsight completion records: {status}"
                    ));
                    return Err(if transient(status) {
                        Retry::Later(error)
                    } else {
                        Retry::No(error)
                    });
                }
                let body = response.bytes().map_err(|e| Retry::Later(failed(e)))?;
                if let Some(progress) = &self.progress {
                    progress.received(body.len() as u64);
                }
                parse_page(&body).map_err(Retry::No)
            })
            .map_err(|e| ZarrError::Open(e.to_string()))?;
            for object in page.contents {
                if object.size > 0
                    && let Some(name) = object
                        .key
                        .strip_prefix(&prefix)
                        .and_then(|n| n.strip_suffix(".json"))
                    && let Ok(chunk) = name.parse::<u64>()
                    && name == chunk.to_string()
                {
                    complete.insert(chunk);
                }
            }
            if !page.is_truncated {
                break;
            }
            token = Some(
                page.next_continuation_token
                    .filter(|t| !t.is_empty() && seen.insert(t.clone()))
                    .ok_or_else(|| {
                        ZarrError::Open(
                            "Hindsight completion listing omitted a new continuation token".into(),
                        )
                    })?,
            );
        }
        Ok(complete)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn paginates_escapes_tokens_and_refreshes_instead_of_reusing_old_dates() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let store = HttpStore::s3(
            &format!(
                "http://127.0.0.1:{}/bucket/history",
                listener.local_addr().unwrap().port()
            ),
            None,
        )
        .unwrap();
        let server = std::thread::spawn(move || {
            let bodies = [
                "<ListBucketResult><IsTruncated>true</IsTruncated><NextContinuationToken>a+/ =&amp;</NextContinuationToken><Contents><Key>history/.historysyncer/complete/9.json</Key><Size>1</Size></Contents><Contents><Key>history/.historysyncer/complete/01.json</Key><Size>1</Size></Contents><Contents><Key>history/.historysyncer/complete/0.json</Key><Size>0</Size></Contents></ListBucketResult>",
                "<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>history/.historysyncer/complete/2.json</Key><Size>1</Size></Contents><Contents><Key>unrelated/1.json</Key><Size>1</Size></Contents></ListBucketResult>",
                "<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>history/.historysyncer/complete/3.json</Key><Size>1</Size></Contents></ListBucketResult>",
            ];
            let mut requests = Vec::new();
            for body in bodies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                requests.push(String::from_utf8(request).unwrap());
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            requests
        });
        assert_eq!(store.completed_chunks().unwrap(), [2, 9].into());
        assert_eq!(store.completed_chunks().unwrap(), [3].into());
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with(
            "GET /bucket/?list-type=2&prefix=history%2F.historysyncer%2Fcomplete%2F "
        ));
        assert!(requests[1].contains("continuation-token=a%2B%2F%20%3D%26&"));
        assert!(!requests[2].contains("continuation-token"));
        assert!(!requests[2].to_ascii_lowercase().contains("if-none-match"));
    }

    #[test]
    fn error_xml_is_not_an_empty_successful_inventory() {
        assert!(parse_page(b"<Error><Code>AccessDenied</Code></Error>").is_err());
        assert!(parse_page(b"<ListBucketResult><Contents>").is_err());
    }
}
