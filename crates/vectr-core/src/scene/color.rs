//! The colour model: a value is a colour when SVG can render it (FEAT-005,
//! FEAT-018).
//!
//! A colour value is written in any format SVG supports, optionally carrying
//! alpha: 3-, 4-, 6-, or 8-digit hexadecimal, `rgb()` and `rgba()`, `hsl()` and
//! `hsla()`, a named colour, or `transparent`. A value that is not such a
//! colour is a located error and no output is produced, never passed through
//! silently for a rasterizer to reject (D-029).
//!
//! The grammar here matches what the SVG rasterizer accepts, so every value
//! that passes validation can be rendered as emitted. The named-colour set is
//! the SVG/CSS one the rasterizer shares.

use super::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics};

/// A value is not a colour SVG supports.
pub const INVALID_COLOR: DiagnosticCode = DiagnosticCode::new("E_INVALID_COLOR");

/// Whether `value` is a colour SVG supports.
///
/// Surrounding whitespace is allowed, as it is in an SVG attribute; the four
/// hexadecimal notations, the `rgb`/`rgba` and `hsl`/`hsla` functions with
/// their comma- or space-separated forms, the named colours, and `transparent`
/// are accepted.
pub fn is_color(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }

    if let Some(hex) = value.strip_prefix('#') {
        return is_hex_color(hex);
    }

    // A function form: `name(...)`. The name must be followed immediately by
    // the opening parenthesis, and the closing parenthesis must end the value.
    if let Some(open) = value.find('(') {
        if !value.ends_with(')') {
            return false;
        }
        let name = value[..open].to_ascii_lowercase();
        let body = &value[open + 1..value.len() - 1];
        return match name.as_str() {
            "rgb" | "rgba" => is_rgb(body),
            "hsl" | "hsla" => is_hsl(body),
            _ => false,
        };
    }

    is_named_color(value)
}

/// Records a located [`INVALID_COLOR`] error when `value` is not a colour.
///
/// `what` names the field or entity the value belongs to, so the finding reads
/// as the author's own term ("canvas background", "palette token `accent`
/// value") rather than an internal field name.
pub fn validate_color(
    diagnostics: &mut Diagnostics,
    value: &str,
    what: &str,
    path: impl Into<String>,
) {
    if !is_color(value) {
        diagnostics.push(
            Diagnostic::error(
                INVALID_COLOR,
                format!("{what} is not a colour SVG supports: `{value}`"),
            )
            .at_path(path),
        );
    }
}

/// Whether the text after `#` is a 3-, 4-, 6-, or 8-digit hexadecimal colour.
fn is_hex_color(hex: &str) -> bool {
    matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Whether a lowercased identifier names a colour.
fn is_named_color(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    NAMED_COLORS.binary_search(&value.as_str()).is_ok()
}

/// Whether an `rgb()` or `rgba()` body holds three colour components and an
/// optional numeric alpha.
fn is_rgb(body: &str) -> bool {
    let Some(parts) = split_components(body) else {
        return false;
    };
    if !(3..=4).contains(&parts.len()) {
        return false;
    }
    parts[..3].iter().all(|part| is_number_or_percent(part)) && {
        match parts.get(3) {
            Some(alpha) => is_number(alpha),
            None => true,
        }
    }
}

/// Whether an `hsl()` or `hsla()` body holds a hue, a saturation and a
/// lightness component, and an optional numeric alpha.
fn is_hsl(body: &str) -> bool {
    let Some(parts) = split_components(body) else {
        return false;
    };
    if !(3..=4).contains(&parts.len()) {
        return false;
    }
    is_number(parts[0])
        && is_number_or_percent(parts[1])
        && is_number_or_percent(parts[2])
        && match parts.get(3) {
            Some(alpha) => is_number(alpha),
            None => true,
        }
}

/// Splits a function body on its list separators.
///
/// Components are separated by any run of spaces, or by a single comma with
/// optional surrounding spaces; an empty component or a trailing separator is
/// refused so `rgb(1,,2,3)` and `rgb(1,2,3,)` are not colours.
fn split_components(body: &str) -> Option<Vec<&str>> {
    let bytes = body.as_bytes();
    let mut parts = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let start = index;
        while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b',' {
            index += 1;
        }
        if start == index {
            return None;
        }
        parts.push(&body[start..index]);

        // A separator is an optional single comma with surrounding spaces; a
        // bare space also separates components.
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index < bytes.len() && bytes[index] == b',' {
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            if index >= bytes.len() {
                return None;
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts)
    }
}

/// Whether a component is a finite number, or a finite percentage.
fn is_number_or_percent(component: &str) -> bool {
    match component.strip_suffix('%') {
        Some(prefix) => is_number(prefix),
        None => is_number(component),
    }
}

/// Whether a component is a finite number.
fn is_number(component: &str) -> bool {
    !component.is_empty()
        && component
            .parse::<f64>()
            .is_ok_and(|value| value.is_finite())
}

/// The SVG/CSS named colours the rasterizer shares, in sorted order.
///
/// `transparent` is a named colour with zero alpha rather than a separate case,
/// so it validates, resolves, and renders through the same path as any other
/// name.
const NAMED_COLORS: [&str; 148] = [
    "aliceblue",
    "antiquewhite",
    "aqua",
    "aquamarine",
    "azure",
    "beige",
    "bisque",
    "black",
    "blanchedalmond",
    "blue",
    "blueviolet",
    "brown",
    "burlywood",
    "cadetblue",
    "chartreuse",
    "chocolate",
    "coral",
    "cornflowerblue",
    "cornsilk",
    "crimson",
    "cyan",
    "darkblue",
    "darkcyan",
    "darkgoldenrod",
    "darkgray",
    "darkgreen",
    "darkgrey",
    "darkkhaki",
    "darkmagenta",
    "darkolivegreen",
    "darkorange",
    "darkorchid",
    "darkred",
    "darksalmon",
    "darkseagreen",
    "darkslateblue",
    "darkslategray",
    "darkslategrey",
    "darkturquoise",
    "darkviolet",
    "deeppink",
    "deepskyblue",
    "dimgray",
    "dimgrey",
    "dodgerblue",
    "firebrick",
    "floralwhite",
    "forestgreen",
    "fuchsia",
    "gainsboro",
    "ghostwhite",
    "gold",
    "goldenrod",
    "gray",
    "green",
    "greenyellow",
    "grey",
    "honeydew",
    "hotpink",
    "indianred",
    "indigo",
    "ivory",
    "khaki",
    "lavender",
    "lavenderblush",
    "lawngreen",
    "lemonchiffon",
    "lightblue",
    "lightcoral",
    "lightcyan",
    "lightgoldenrodyellow",
    "lightgray",
    "lightgreen",
    "lightgrey",
    "lightpink",
    "lightsalmon",
    "lightseagreen",
    "lightskyblue",
    "lightslategray",
    "lightslategrey",
    "lightsteelblue",
    "lightyellow",
    "lime",
    "limegreen",
    "linen",
    "magenta",
    "maroon",
    "mediumaquamarine",
    "mediumblue",
    "mediumorchid",
    "mediumpurple",
    "mediumseagreen",
    "mediumslateblue",
    "mediumspringgreen",
    "mediumturquoise",
    "mediumvioletred",
    "midnightblue",
    "mintcream",
    "mistyrose",
    "moccasin",
    "navajowhite",
    "navy",
    "oldlace",
    "olive",
    "olivedrab",
    "orange",
    "orangered",
    "orchid",
    "palegoldenrod",
    "palegreen",
    "paleturquoise",
    "palevioletred",
    "papayawhip",
    "peachpuff",
    "peru",
    "pink",
    "plum",
    "powderblue",
    "purple",
    "red",
    "rosybrown",
    "royalblue",
    "saddlebrown",
    "salmon",
    "sandybrown",
    "seagreen",
    "seashell",
    "sienna",
    "silver",
    "skyblue",
    "slateblue",
    "slategray",
    "slategrey",
    "snow",
    "springgreen",
    "steelblue",
    "tan",
    "teal",
    "thistle",
    "tomato",
    "transparent",
    "turquoise",
    "violet",
    "wheat",
    "white",
    "whitesmoke",
    "yellow",
    "yellowgreen",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_every_hexadecimal_notation() {
        for value in ["#f00", "#f008", "#ff0000", "#ff000080", "#ABC", "#aBcD12Ef"] {
            assert!(is_color(value), "expected {value:?} to be a colour");
        }
    }

    #[test]
    fn rejects_a_hexadecimal_value_of_the_wrong_length_or_alphabet() {
        for value in [
            "#", "#ff", "#fffff", "#fffffff", "#gg0000", "#12 34", "ff0000",
        ] {
            assert!(!is_color(value), "expected {value:?} to be refused");
        }
    }

    #[test]
    fn accepts_rgb_and_rgba_in_both_separators() {
        for value in [
            "rgb(255, 0, 0)",
            "rgb(255 0 0)",
            "rgb(100%, 0%, 50%)",
            "rgba(1, 2, 3, 0.5)",
            "rgb(1,2,3,0.5)",
            "RGB(255,0,0)",
            " rgb( 77 , 77 , 77 ) ",
        ] {
            assert!(is_color(value), "expected {value:?} to be a colour");
        }
    }

    #[test]
    fn rejects_rgb_that_is_not_three_or_four_components() {
        for value in [
            "rgb(1, 2)",
            "rgb(1, 2, 3, 0.5, 6)",
            "rgb(1,,2,3)",
            "rgb(1, 2, 3,)",
            "rgb()",
            "rgb(1, 2, 3",
            "rgb 1,2,3",
            "rgb(1, 2, 3, 50%)",
            "rgb(1, 2, red)",
        ] {
            assert!(!is_color(value), "expected {value:?} to be refused");
        }
    }

    #[test]
    fn accepts_hsl_and_hsla() {
        for value in [
            "hsl(120, 50%, 50%)",
            "hsl(120 50% 50%)",
            "hsla(120, 50%, 50%, 0.25)",
            "hsl(-30, 1, 1)",
        ] {
            assert!(is_color(value), "expected {value:?} to be a colour");
        }
    }

    #[test]
    fn rejects_hsl_without_a_plain_hue() {
        for value in ["hsl(50%, 50%, 50%)", "hsl(120, 50, 50, 0.5, 1)", "hsl()"] {
            assert!(!is_color(value), "expected {value:?} to be refused");
        }
    }

    #[test]
    fn accepts_named_colours_case_insensitively() {
        for value in ["red", "Red", "transparent", "TRANSPARENT", "gray", "grey"] {
            assert!(is_color(value), "expected {value:?} to be a colour");
        }
    }

    #[test]
    fn rejects_arbitrary_identifiers_and_empty_values() {
        // `rebeccapurple` is CSS Color 4, which the SVG rasterizer's named set
        // does not carry, so it is refused rather than passed through.
        for value in [
            "",
            "  ",
            "notacolour",
            "none",
            "currentColor",
            "rebeccapurple",
            "12",
        ] {
            assert!(!is_color(value), "expected {value:?} to be refused");
        }
    }

    #[test]
    fn validate_color_records_a_located_error_naming_the_value() {
        let mut diagnostics = Diagnostics::new();
        validate_color(
            &mut diagnostics,
            "#fff",
            "canvas background",
            "/canvas/background",
        );
        assert!(diagnostics.is_empty());

        let mut diagnostics = Diagnostics::new();
        validate_color(
            &mut diagnostics,
            "#gg",
            "canvas background",
            "/canvas/background",
        );
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, INVALID_COLOR);
        assert!(error.message.contains("canvas background"));
        assert!(error.message.contains("#gg"));
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/canvas/background")
        );
    }
}
