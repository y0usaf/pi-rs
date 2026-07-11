//! Port of `utils/headers.ts`.

use std::collections::BTreeMap;

use reqwest::header::HeaderMap;

/// Spec: `headersToRecord(headers: Headers)`. Keys come out lowercase (as
/// the Fetch `Headers` iterator yields them; reqwest's `HeaderName` is
/// already lowercase) and repeated headers are joined with `", "`, which
/// is how Fetch combines duplicates.
pub fn headers_to_record(headers: &HeaderMap) -> BTreeMap<String, String> {
    let mut record: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in headers {
        let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
        record
            .entry(name.as_str().to_string())
            .and_modify(|existing| {
                existing.push_str(", ");
                existing.push_str(&value);
            })
            .or_insert(value);
    }
    record
}
