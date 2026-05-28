//! Project-identity stamp drawn in the bottom-right corner of every
//! rendered view.
//!
//! Carries the project's name and version (the manifest's required
//! fields), plus revision and date when set. Renderers extend their
//! viewBox height by [`STAMP_HEIGHT`] when a manifest is present, then
//! emit a `<text class="stamp">` element through [`stamp_element`].

use svg::node::element::Text;

use crate::dsl::manifest::Manifest;

/// Vertical room (world units) added to a view's viewBox to leave space
/// for the corner stamp.
pub(crate) const STAMP_HEIGHT: f64 = 22.0;

/// Inset from the viewBox's right/bottom edge to the stamp text baseline.
pub(crate) const STAMP_INSET: f64 = 8.0;

/// Build the human-readable stamp string: `"<name> v<version>"`, then
/// `" · rev <revision>"` and `" (<date>)"` when those fields are set.
pub(crate) fn stamp_text(manifest: &Manifest) -> String {
    let mut s = format!("{} v{}", manifest.name, manifest.version);
    if let Some(rev) = &manifest.revision {
        s.push_str(" · rev ");
        s.push_str(rev);
    }
    if let Some(date) = manifest.date {
        s.push_str(&format!(" ({date})"));
    }
    s
}

/// A positioned `<text class="stamp">` element. `right`/`bottom` are the
/// stamp's anchor — the text right-aligns to `right` and sits on
/// `bottom` as its baseline.
pub(crate) fn stamp_element(manifest: &Manifest, right: f64, bottom: f64) -> Text {
    Text::new(stamp_text(manifest))
        .set("class", "stamp")
        .set("x", right)
        .set("y", bottom)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn minimal() -> Manifest {
        Manifest {
            name: "demo".to_string(),
            version: "1.2.3".to_string(),
            description: None,
            authors: Vec::new(),
            license: None,
            revision: None,
            date: None,
        }
    }

    #[test]
    fn name_and_version_only() {
        assert_eq!(stamp_text(&minimal()), "demo v1.2.3");
    }

    #[test]
    fn revision_appended_when_present() {
        let m = Manifest {
            revision: Some("abc1234".to_string()),
            ..minimal()
        };
        assert_eq!(stamp_text(&m), "demo v1.2.3 · rev abc1234");
    }

    #[test]
    fn dirty_marker_passes_through_revision_string() {
        let m = Manifest {
            revision: Some("abc1234-dirty".to_string()),
            ..minimal()
        };
        assert_eq!(stamp_text(&m), "demo v1.2.3 · rev abc1234-dirty");
    }

    #[test]
    fn date_appended_when_present() {
        let m = Manifest {
            date: Some(NaiveDate::from_ymd_opt(2026, 5, 28).unwrap()),
            ..minimal()
        };
        assert_eq!(stamp_text(&m), "demo v1.2.3 (2026-05-28)");
    }

    #[test]
    fn all_fields_present() {
        let m = Manifest {
            revision: Some("B".to_string()),
            date: Some(NaiveDate::from_ymd_opt(2026, 5, 28).unwrap()),
            ..minimal()
        };
        assert_eq!(stamp_text(&m), "demo v1.2.3 · rev B (2026-05-28)");
    }
}
