//! File-path ↔ `file://` URI conversion.
//!
//! `lsp-types` 0.97 carries URIs as plain validated strings (via
//! `fluent-uri`) without the `url` crate's filesystem helpers, so the
//! round-trip is ours. Only absolute paths in UTF-8 are supported — the
//! VSCode client's document selector is `scheme: "file"`, and the DSL
//! pipeline already assumes printable paths.
//!
//! On Windows, a drive path `C:\dev\main.wb` maps to
//! `file:///c:/dev/main.wb` (lowercase drive in the URI, matching what
//! VSCode sends; uppercase drive in the decoded path), and the
//! `\\?\`-verbatim prefix that `fs::canonicalize` adds is stripped before
//! encoding. UNC paths (`\\server\share`) are not supported, mirroring the
//! decoder's rejection of URIs with a host authority.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use lsp_types::Uri;

/// RFC 3986 unreserved characters, plus `/` as the path separator.
fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/')
}

/// Encode an absolute path as a `file://` URI.
pub(crate) fn to_uri(path: &Path) -> Option<Uri> {
    Uri::from_str(&encode(path.to_str()?, cfg!(windows))?).ok()
}

/// Decode a `file://` URI back to a path. Returns `None` for any other
/// scheme, a non-empty authority, or invalid percent-encoding.
pub(crate) fn to_path(uri: &Uri) -> Option<PathBuf> {
    Some(PathBuf::from(decode(uri.as_str(), cfg!(windows))?))
}

/// Encode an absolute path string as a `file://` URI string. `windows`
/// selects the path flavor, so both arms stay testable from any host.
fn encode(path: &str, windows: bool) -> Option<String> {
    let mut s = String::with_capacity(path.len() + 8);
    s.push_str("file://");
    let rest = if windows {
        let path = path.strip_prefix(r"\\?\").unwrap_or(path);
        let bytes = path.as_bytes();
        // Drive-absolute only: `C:\…` or `C:/…`. UNC paths are rejected.
        if !(bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/'))
        {
            return None;
        }
        s.push('/');
        s.push(bytes[0].to_ascii_lowercase() as char);
        s.push(':');
        &path[2..]
    } else {
        if !path.starts_with('/') {
            return None;
        }
        path
    };
    for &b in rest.as_bytes() {
        let b = if windows && b == b'\\' { b'/' } else { b };
        if is_unreserved(b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    Some(s)
}

/// Decode a `file://` URI string to an absolute path string in the given
/// flavor.
fn decode(uri: &str, windows: bool) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    if !rest.starts_with('/') {
        return None; // a host authority (file://server/…) is not ours
    }
    let mut bytes = Vec::with_capacity(rest.len());
    let mut iter = rest.bytes();
    while let Some(b) = iter.next() {
        if b == b'%' {
            let hex = [iter.next()?, iter.next()?];
            let hex = std::str::from_utf8(&hex).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
        } else {
            bytes.push(b);
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    if !windows {
        return Some(path);
    }
    // `/c:/dev/…` (drive case and `:` vs `%3A` both vary by client) →
    // `C:\dev\…`.
    let stripped = path.strip_prefix('/')?;
    let bytes = stripped.as_bytes();
    if !(bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':') {
        return None;
    }
    if bytes.len() > 2 && bytes[2] != b'/' {
        return None;
    }
    let mut out = String::with_capacity(stripped.len() + 1);
    out.push(bytes[0].to_ascii_uppercase() as char);
    out.push(':');
    if stripped.len() == 2 {
        out.push('\\'); // bare drive: `file:///c:` → `C:\`
    }
    out.extend(
        stripped[2..]
            .chars()
            .map(|c| if c == '/' { '\\' } else { c }),
    );
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
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
    #[cfg(windows)]
    fn plain_path_round_trips() {
        let path = Path::new(r"C:\dev\wirebug\examples\main.wb");
        let uri = to_uri(path).expect("encodes");
        assert_eq!(uri.as_str(), "file:///c:/dev/wirebug/examples/main.wb");
        assert_eq!(to_path(&uri).expect("decodes"), path);
    }

    #[test]
    fn space_and_unicode_round_trip() {
        let uri = encode("/home/user/My Projects/blåbär.wb", false).expect("encodes");
        assert!(uri.contains("My%20Projects"), "{uri}");
        assert_eq!(
            decode(&uri, false).expect("decodes"),
            "/home/user/My Projects/blåbär.wb"
        );
    }

    #[test]
    fn vscode_style_uri_decodes() {
        assert_eq!(
            decode("file:///home/lhelge/dev/wirebug/examples/main.wb", false).expect("decodes"),
            "/home/lhelge/dev/wirebug/examples/main.wb"
        );
    }

    #[test]
    fn relative_path_is_rejected() {
        assert_eq!(encode("dev/wirebug/main.wb", false), None);
        assert_eq!(encode(r"dev\wirebug\main.wb", true), None);
    }

    #[test]
    fn non_file_scheme_is_rejected() {
        let uri = Uri::from_str("untitled:Untitled-1").unwrap();
        assert_eq!(to_path(&uri), None);
    }

    #[test]
    fn windows_drive_path_encodes() {
        assert_eq!(
            encode(r"C:\dev\wirebug\examples\main.wb", true).expect("encodes"),
            "file:///c:/dev/wirebug/examples/main.wb"
        );
    }

    #[test]
    fn windows_verbatim_prefix_is_stripped() {
        assert_eq!(
            encode(r"\\?\C:\dev\wirebug\main.wb", true).expect("encodes"),
            "file:///c:/dev/wirebug/main.wb"
        );
    }

    #[test]
    fn windows_unc_path_is_rejected() {
        assert_eq!(encode(r"\\server\share\main.wb", true), None);
    }

    #[test]
    fn vscode_windows_uri_decodes() {
        // VSCode percent-encodes the drive colon and lowercases the letter.
        assert_eq!(
            decode("file:///c%3A/dev/wirebug/examples/main.wb", true).expect("decodes"),
            r"C:\dev\wirebug\examples\main.wb"
        );
    }

    #[test]
    fn plain_drive_colon_decodes() {
        assert_eq!(
            decode("file:///C:/dev/main.wb", true).expect("decodes"),
            r"C:\dev\main.wb"
        );
    }

    #[test]
    fn bare_drive_decodes_to_root() {
        assert_eq!(decode("file:///c%3A", true).expect("decodes"), r"C:\");
    }

    #[test]
    fn windows_uri_without_drive_is_rejected() {
        assert_eq!(decode("file:///home/user/main.wb", true), None);
    }

    #[test]
    fn windows_round_trips() {
        let path = r"C:\Users\lhelge\My Projects\blåbär.wb";
        let uri = encode(path, true).expect("encodes");
        assert_eq!(decode(&uri, true).expect("decodes"), path);
    }

    #[test]
    fn space_and_unicode_uri_is_valid() {
        let path = Path::new("/home/user/My Projects/blåbär.wb");
        let uri = to_uri(path).expect("encodes");
        assert_eq!(to_path(&uri).expect("decodes"), path);
    }
}
