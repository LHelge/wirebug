//! Wire-color abbreviations for annotations.

/// The IEC 60757 code for a wire color authored as a CSS color name
/// (`"white"` → `"WH"`). A color outside the standard set passes through
/// verbatim, so exotic names stay readable rather than vanishing.
pub(crate) fn iec_code(color: &str) -> &str {
    match color.to_ascii_lowercase().as_str() {
        "black" => "BK",
        "brown" => "BN",
        "red" => "RD",
        "orange" => "OG",
        "yellow" => "YE",
        "green" => "GN",
        "blue" => "BU",
        "violet" | "purple" => "VT",
        "gray" | "grey" => "GY",
        "white" => "WH",
        "pink" => "PK",
        "turquoise" => "TQ",
        _ => color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_colors_map_to_iec_codes() {
        assert_eq!(iec_code("white"), "WH");
        assert_eq!(iec_code("green"), "GN");
        assert_eq!(iec_code("gray"), "GY");
        assert_eq!(iec_code("grey"), "GY");
        assert_eq!(iec_code("purple"), "VT");
    }

    #[test]
    fn matching_ignores_case() {
        assert_eq!(iec_code("White"), "WH");
        assert_eq!(iec_code("RED"), "RD");
    }

    #[test]
    fn unknown_color_passes_through() {
        assert_eq!(iec_code("chartreuse"), "chartreuse");
    }
}
