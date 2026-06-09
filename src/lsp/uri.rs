//! File-path ↔ `file://` URI conversion.
//!
//! `lsp-types` 0.97 carries URIs as plain validated strings (via
//! `fluent-uri`) without the `url` crate's filesystem helpers, so the
//! round-trip is ours. Only absolute `file://` paths in UTF-8 are
//! supported — the VSCode client's document selector is `scheme: "file"`,
//! and the DSL pipeline already assumes printable paths.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use lsp_types::Uri;

/// RFC 3986 unreserved characters, plus `/` as the path separator.
fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/')
}

/// Encode an absolute path as a `file://` URI.
pub(crate) fn to_uri(path: &Path) -> Option<Uri> {
    let path = path.to_str()?;
    let mut s = String::with_capacity(path.len() + 8);
    s.push_str("file://");
    for &b in path.as_bytes() {
        if is_unreserved(b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    Uri::from_str(&s).ok()
}

/// Decode a `file://` URI back to a path. Returns `None` for any other
/// scheme, a non-empty authority, or invalid percent-encoding.
pub(crate) fn to_path(uri: &Uri) -> Option<PathBuf> {
    let rest = uri.as_str().strip_prefix("file://")?;
    if !rest.starts_with('/') {
        return None; // a host authority (file://server/…) is not ours
    }
    let mut bytes = Vec::with_capacity(rest.len());
    let mut iter = rest.bytes();
    while let Some(b) = iter.next() {
        if b == b'%' {
            let hi = iter.next()?;
            let lo = iter.next()?;
            let hex = [hi, lo];
            let hex = std::str::from_utf8(&hex).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
        } else {
            bytes.push(b);
        }
    }
    Some(PathBuf::from(String::from_utf8(bytes).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_path_round_trips() {
        let path = Path::new("/home/user/dev/wirebug/examples/main.wb");
        let uri = to_uri(path).expect("encodes");
        assert_eq!(
            uri.as_str(),
            "file:///home/user/dev/wirebug/examples/main.wb"
        );
        assert_eq!(to_path(&uri).expect("decodes"), path);
    }

    #[test]
    fn space_and_unicode_round_trip() {
        let path = Path::new("/home/user/My Projects/blåbär.wb");
        let uri = to_uri(path).expect("encodes");
        assert!(uri.as_str().contains("My%20Projects"), "{uri:?}");
        assert_eq!(to_path(&uri).expect("decodes"), path);
    }

    #[test]
    fn vscode_style_uri_decodes() {
        let uri = Uri::from_str("file:///home/lhelge/dev/wirebug/examples/main.wb").unwrap();
        assert_eq!(
            to_path(&uri).expect("decodes"),
            Path::new("/home/lhelge/dev/wirebug/examples/main.wb")
        );
    }

    #[test]
    fn non_file_scheme_is_rejected() {
        let uri = Uri::from_str("untitled:Untitled-1").unwrap();
        assert_eq!(to_path(&uri), None);
    }
}
