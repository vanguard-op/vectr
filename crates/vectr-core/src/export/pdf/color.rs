//! Colour resolution for PDF output (FEAT-014).
//!
//! PDF has no textual colour syntax: every paint must become concrete device
//! components. This module turns any colour the scene language accepts — the
//! hexadecimal notations, `rgb`/`rgba`, `hsl`/`hsla`, a named colour, or
//! `transparent` — into device RGB plus an alpha value, so the vector emitter
//! writes the matching operator without depending on the rasterizer (FEAT-018).

/// A resolved colour: red, green, blue and alpha, each in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rgba {
    pub(crate) r: f64,
    pub(crate) g: f64,
    pub(crate) b: f64,
    pub(crate) a: f64,
}

/// Opaque white, the paper a print profile composites against.
pub(crate) const WHITE: Rgba = Rgba {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};

/// Opaque black, the fallback for a value that cannot be resolved.
pub(crate) const BLACK: Rgba = Rgba {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

impl Rgba {
    /// Composites this colour over an opaque backdrop, yielding an opaque
    /// colour. Used where a target cannot carry per-paint transparency.
    pub(crate) fn over(self, backdrop: Rgba) -> Rgba {
        let a = self.a.clamp(0.0, 1.0);
        Rgba {
            r: self.r * a + backdrop.r * (1.0 - a),
            g: self.g * a + backdrop.g * (1.0 - a),
            b: self.b * a + backdrop.b * (1.0 - a),
            a: 1.0,
        }
    }
}

/// Resolves a colour value the scene language accepts, or `None` when it is not
/// one. The value is assumed to have passed the engine's colour validation; a
/// malformed value is refused rather than guessed at.
pub(crate) fn parse(value: &str) -> Option<Rgba> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(hex) = value.strip_prefix('#') {
        return parse_hex(hex);
    }
    if let Some(open) = value.find('(') {
        if !value.ends_with(')') {
            return None;
        }
        let name = value[..open].to_ascii_lowercase();
        let body = &value[open + 1..value.len() - 1];
        return match name.as_str() {
            "rgb" | "rgba" => parse_rgb(body),
            "hsl" | "hsla" => parse_hsl(body),
            _ => None,
        };
    }
    named(value)
}

/// Parses a 3-, 4-, 6- or 8-digit hexadecimal colour.
fn parse_hex(hex: &str) -> Option<Rgba> {
    let digits: Vec<u32> = hex
        .chars()
        .map(|ch| ch.to_digit(16))
        .collect::<Option<Vec<_>>>()?;
    let (r, g, b, a) = match digits.len() {
        3 => (digits[0] * 17, digits[1] * 17, digits[2] * 17, 255),
        4 => (
            digits[0] * 17,
            digits[1] * 17,
            digits[2] * 17,
            digits[3] * 17,
        ),
        6 => (
            digits[0] * 16 + digits[1],
            digits[2] * 16 + digits[3],
            digits[4] * 16 + digits[5],
            255,
        ),
        8 => (
            digits[0] * 16 + digits[1],
            digits[2] * 16 + digits[3],
            digits[4] * 16 + digits[5],
            digits[6] * 16 + digits[7],
        ),
        _ => return None,
    };
    Some(Rgba {
        r: f64::from(r) / 255.0,
        g: f64::from(g) / 255.0,
        b: f64::from(b) / 255.0,
        a: f64::from(a) / 255.0,
    })
}

/// Parses an `rgb()`/`rgba()` body, whose components are numbers or percentages.
fn parse_rgb(body: &str) -> Option<Rgba> {
    let parts = components(body)?;
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let channel = |part: &str| -> Option<f64> {
        match part.strip_suffix('%') {
            Some(prefix) => number(prefix).map(|value| value / 100.0),
            None => number(part).map(|value| value / 255.0),
        }
    };
    let r = channel(parts[0])?;
    let g = channel(parts[1])?;
    let b = channel(parts[2])?;
    let a = match parts.get(3) {
        Some(alpha) => number(alpha)?,
        None => 1.0,
    };
    Some(Rgba {
        r: r.clamp(0.0, 1.0),
        g: g.clamp(0.0, 1.0),
        b: b.clamp(0.0, 1.0),
        a: a.clamp(0.0, 1.0),
    })
}

/// Parses an `hsl()`/`hsla()` body: a hue in degrees and saturation and
/// lightness as percentages or fractions.
fn parse_hsl(body: &str) -> Option<Rgba> {
    let parts = components(body)?;
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let hue = number(parts[0])?;
    let fraction = |part: &str| -> Option<f64> {
        match part.strip_suffix('%') {
            Some(prefix) => number(prefix).map(|value| value / 100.0),
            None => number(part),
        }
    };
    let saturation = fraction(parts[1])?;
    let lightness = fraction(parts[2])?;
    let a = match parts.get(3) {
        Some(alpha) => number(alpha)?,
        None => 1.0,
    };
    let (r, g, b) = hsl_to_rgb(hue, saturation.clamp(0.0, 1.0), lightness.clamp(0.0, 1.0));
    Some(Rgba {
        r,
        g,
        b,
        a: a.clamp(0.0, 1.0),
    })
}

/// Splits a function body on commas and whitespace.
fn components(body: &str) -> Option<Vec<&str>> {
    let parts: Vec<&str> = body
        .split(|ch: char| ch == ',' || ch.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts)
    }
}

fn number(component: &str) -> Option<f64> {
    component
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

/// Converts an HSL colour to RGB, each component in `0.0..=1.0`.
fn hsl_to_rgb(hue: f64, saturation: f64, lightness: f64) -> (f64, f64, f64) {
    if saturation == 0.0 {
        return (lightness, lightness, lightness);
    }
    let hue = ((hue % 360.0) + 360.0) % 360.0 / 360.0;
    let q = if lightness < 0.5 {
        lightness * (1.0 + saturation)
    } else {
        lightness + saturation - lightness * saturation
    };
    let p = 2.0 * lightness - q;
    let channel = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (
        channel(hue + 1.0 / 3.0),
        channel(hue),
        channel(hue - 1.0 / 3.0),
    )
}

/// Looks up a named colour, case-insensitively.
fn named(value: &str) -> Option<Rgba> {
    let lower = value.to_ascii_lowercase();
    NAMED_RGB
        .binary_search_by_key(&lower.as_str(), |(name, _, _, _, _)| name)
        .ok()
        .map(|index| {
            let (_, r, g, b, a) = NAMED_RGB[index];
            Rgba {
                r: f64::from(r) / 255.0,
                g: f64::from(g) / 255.0,
                b: f64::from(b) / 255.0,
                a: f64::from(a) / 255.0,
            }
        })
}

/// The SVG/CSS named colours, sorted by name for binary search.
const NAMED_RGB: [(&str, u8, u8, u8, u8); 148] = [
    ("aliceblue", 240, 248, 255, 255),
    ("antiquewhite", 250, 235, 215, 255),
    ("aqua", 0, 255, 255, 255),
    ("aquamarine", 127, 255, 212, 255),
    ("azure", 240, 255, 255, 255),
    ("beige", 245, 245, 220, 255),
    ("bisque", 255, 228, 196, 255),
    ("black", 0, 0, 0, 255),
    ("blanchedalmond", 255, 235, 205, 255),
    ("blue", 0, 0, 255, 255),
    ("blueviolet", 138, 43, 226, 255),
    ("brown", 165, 42, 42, 255),
    ("burlywood", 222, 184, 135, 255),
    ("cadetblue", 95, 158, 160, 255),
    ("chartreuse", 127, 255, 0, 255),
    ("chocolate", 210, 105, 30, 255),
    ("coral", 255, 127, 80, 255),
    ("cornflowerblue", 100, 149, 237, 255),
    ("cornsilk", 255, 248, 220, 255),
    ("crimson", 220, 20, 60, 255),
    ("cyan", 0, 255, 255, 255),
    ("darkblue", 0, 0, 139, 255),
    ("darkcyan", 0, 139, 139, 255),
    ("darkgoldenrod", 184, 134, 11, 255),
    ("darkgray", 169, 169, 169, 255),
    ("darkgreen", 0, 100, 0, 255),
    ("darkgrey", 169, 169, 169, 255),
    ("darkkhaki", 189, 183, 107, 255),
    ("darkmagenta", 139, 0, 139, 255),
    ("darkolivegreen", 85, 107, 47, 255),
    ("darkorange", 255, 140, 0, 255),
    ("darkorchid", 153, 50, 204, 255),
    ("darkred", 139, 0, 0, 255),
    ("darksalmon", 233, 150, 122, 255),
    ("darkseagreen", 143, 188, 143, 255),
    ("darkslateblue", 72, 61, 139, 255),
    ("darkslategray", 47, 79, 79, 255),
    ("darkslategrey", 47, 79, 79, 255),
    ("darkturquoise", 0, 206, 209, 255),
    ("darkviolet", 148, 0, 211, 255),
    ("deeppink", 255, 20, 147, 255),
    ("deepskyblue", 0, 191, 255, 255),
    ("dimgray", 105, 105, 105, 255),
    ("dimgrey", 105, 105, 105, 255),
    ("dodgerblue", 30, 144, 255, 255),
    ("firebrick", 178, 34, 34, 255),
    ("floralwhite", 255, 250, 240, 255),
    ("forestgreen", 34, 139, 34, 255),
    ("fuchsia", 255, 0, 255, 255),
    ("gainsboro", 220, 220, 220, 255),
    ("ghostwhite", 248, 248, 255, 255),
    ("gold", 255, 215, 0, 255),
    ("goldenrod", 218, 165, 32, 255),
    ("gray", 128, 128, 128, 255),
    ("green", 0, 128, 0, 255),
    ("greenyellow", 173, 255, 47, 255),
    ("grey", 128, 128, 128, 255),
    ("honeydew", 240, 255, 240, 255),
    ("hotpink", 255, 105, 180, 255),
    ("indianred", 205, 92, 92, 255),
    ("indigo", 75, 0, 130, 255),
    ("ivory", 255, 255, 240, 255),
    ("khaki", 240, 230, 140, 255),
    ("lavender", 230, 230, 250, 255),
    ("lavenderblush", 255, 240, 245, 255),
    ("lawngreen", 124, 252, 0, 255),
    ("lemonchiffon", 255, 250, 205, 255),
    ("lightblue", 173, 216, 230, 255),
    ("lightcoral", 240, 128, 128, 255),
    ("lightcyan", 224, 255, 255, 255),
    ("lightgoldenrodyellow", 250, 250, 210, 255),
    ("lightgray", 211, 211, 211, 255),
    ("lightgreen", 144, 238, 144, 255),
    ("lightgrey", 211, 211, 211, 255),
    ("lightpink", 255, 182, 193, 255),
    ("lightsalmon", 255, 160, 122, 255),
    ("lightseagreen", 32, 178, 170, 255),
    ("lightskyblue", 135, 206, 250, 255),
    ("lightslategray", 119, 136, 153, 255),
    ("lightslategrey", 119, 136, 153, 255),
    ("lightsteelblue", 176, 196, 222, 255),
    ("lightyellow", 255, 255, 224, 255),
    ("lime", 0, 255, 0, 255),
    ("limegreen", 50, 205, 50, 255),
    ("linen", 250, 240, 230, 255),
    ("magenta", 255, 0, 255, 255),
    ("maroon", 128, 0, 0, 255),
    ("mediumaquamarine", 102, 205, 170, 255),
    ("mediumblue", 0, 0, 205, 255),
    ("mediumorchid", 186, 85, 211, 255),
    ("mediumpurple", 147, 112, 219, 255),
    ("mediumseagreen", 60, 179, 113, 255),
    ("mediumslateblue", 123, 104, 238, 255),
    ("mediumspringgreen", 0, 250, 154, 255),
    ("mediumturquoise", 72, 209, 204, 255),
    ("mediumvioletred", 199, 21, 133, 255),
    ("midnightblue", 25, 25, 112, 255),
    ("mintcream", 245, 255, 250, 255),
    ("mistyrose", 255, 228, 225, 255),
    ("moccasin", 255, 228, 181, 255),
    ("navajowhite", 255, 222, 173, 255),
    ("navy", 0, 0, 128, 255),
    ("oldlace", 253, 245, 230, 255),
    ("olive", 128, 128, 0, 255),
    ("olivedrab", 107, 142, 35, 255),
    ("orange", 255, 165, 0, 255),
    ("orangered", 255, 69, 0, 255),
    ("orchid", 218, 112, 214, 255),
    ("palegoldenrod", 238, 232, 170, 255),
    ("palegreen", 152, 251, 152, 255),
    ("paleturquoise", 175, 238, 238, 255),
    ("palevioletred", 219, 112, 147, 255),
    ("papayawhip", 255, 239, 213, 255),
    ("peachpuff", 255, 218, 185, 255),
    ("peru", 205, 133, 63, 255),
    ("pink", 255, 192, 203, 255),
    ("plum", 221, 160, 221, 255),
    ("powderblue", 176, 224, 230, 255),
    ("purple", 128, 0, 128, 255),
    ("red", 255, 0, 0, 255),
    ("rosybrown", 188, 143, 143, 255),
    ("royalblue", 65, 105, 225, 255),
    ("saddlebrown", 139, 69, 19, 255),
    ("salmon", 250, 128, 114, 255),
    ("sandybrown", 244, 164, 96, 255),
    ("seagreen", 46, 139, 87, 255),
    ("seashell", 255, 245, 238, 255),
    ("sienna", 160, 82, 45, 255),
    ("silver", 192, 192, 192, 255),
    ("skyblue", 135, 206, 235, 255),
    ("slateblue", 106, 90, 205, 255),
    ("slategray", 112, 128, 144, 255),
    ("slategrey", 112, 128, 144, 255),
    ("snow", 255, 250, 250, 255),
    ("springgreen", 0, 255, 127, 255),
    ("steelblue", 70, 130, 180, 255),
    ("tan", 210, 180, 140, 255),
    ("teal", 0, 128, 128, 255),
    ("thistle", 216, 191, 216, 255),
    ("tomato", 255, 99, 71, 255),
    ("transparent", 0, 0, 0, 0),
    ("turquoise", 64, 224, 208, 255),
    ("violet", 238, 130, 238, 255),
    ("wheat", 245, 222, 179, 255),
    ("white", 255, 255, 255, 255),
    ("whitesmoke", 245, 245, 245, 255),
    ("yellow", 255, 255, 0, 255),
    ("yellowgreen", 154, 205, 50, 255),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn close(value: f64, expected: f64) {
        assert!(
            (value - expected).abs() < 1e-6,
            "expected {expected}, got {value}"
        );
    }

    #[test]
    fn hexadecimal_forms_resolve_to_components_and_alpha() {
        let short = parse("#f00").expect("a colour");
        close(short.r, 1.0);
        close(short.g, 0.0);
        close(short.a, 1.0);

        let eight = parse("#ff000080").expect("a colour");
        close(eight.r, 1.0);
        close(eight.a, 128.0 / 255.0);

        let four = parse("#0f08").expect("a colour");
        close(four.g, 1.0);
        close(four.a, 136.0 / 255.0);
    }

    #[test]
    fn rgb_and_rgba_resolve_numbers_and_percentages() {
        let rgb = parse("rgb(255, 0, 0)").expect("a colour");
        close(rgb.r, 1.0);
        close(rgb.a, 1.0);

        let percent = parse("rgb(100% 0% 50%)").expect("a colour");
        close(percent.r, 1.0);
        close(percent.b, 0.5);

        let rgba = parse("rgba(1, 2, 3, 0.5)").expect("a colour");
        close(rgba.a, 0.5);
    }

    #[test]
    fn hsl_resolves_through_the_colour_wheel() {
        let red = parse("hsl(0, 100%, 50%)").expect("a colour");
        close(red.r, 1.0);
        close(red.g, 0.0);
        close(red.b, 0.0);

        let green = parse("hsl(120 100% 50%)").expect("a colour");
        close(green.g, 1.0);

        let grey = parse("hsl(0, 0%, 50%)").expect("a colour");
        close(grey.r, 0.5);
        close(grey.g, 0.5);
    }

    #[test]
    fn named_colours_resolve_case_insensitively() {
        let red = parse("Red").expect("a colour");
        close(red.r, 1.0);
        close(red.g, 0.0);

        let transparent = parse("transparent").expect("a colour");
        close(transparent.a, 0.0);

        assert_eq!(parse("rebeccapurple"), None);
        assert_eq!(parse("not-a-colour"), None);
    }

    #[test]
    fn compositing_over_a_backdrop_yields_an_opaque_colour() {
        let half_black = Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.5,
        };
        let result = half_black.over(WHITE);
        close(result.r, 0.5);
        close(result.a, 1.0);
    }

    #[test]
    fn the_named_table_is_sorted_for_binary_search() {
        for window in NAMED_RGB.windows(2) {
            assert!(
                window[0].0 < window[1].0,
                "{} >= {}",
                window[0].0,
                window[1].0
            );
        }
    }
}
