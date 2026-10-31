//! Reading CSS without a server: where selectors and declarations are, how
//! specific a selector is, and what its colors are, in which gamut.

/// What a stretch of a style sheet between `{`, `}` and `;` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentKind {
    /// Before a `{`: a selector list or an at-rule.
    Prelude,
    /// Before a `;` or `}`: `property: value`.
    Declaration,
}

/// Char offsets of a stretch, without surrounding white space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub kind: SegmentKind,
    pub start: usize,
    pub end: usize,
}

/// Index past a comment or string starting at `i`, if one does.
fn skip_opaque(chars: &[char], i: usize) -> Option<usize> {
    match chars[i] {
        '/' if chars.get(i + 1) == Some(&'*') => {
            let mut j = i + 2;
            while j + 1 < chars.len() && !(chars[j] == '*' && chars[j + 1] == '/') {
                j += 1;
            }
            Some((j + 2).min(chars.len()))
        }
        // Sass and Less. Not after a colon, which is `http://`.
        '/' if chars.get(i + 1) == Some(&'/') && (i == 0 || chars[i - 1] != ':') => {
            let mut j = i;
            while j < chars.len() && chars[j] != '\n' {
                j += 1;
            }
            Some(j)
        }
        quote @ ('"' | '\'') => {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != quote && chars[j] != '\n' {
                j += if chars[j] == '\\' { 2 } else { 1 };
            }
            Some((j + 1).min(chars.len()))
        }
        _ => None,
    }
}

/// The stretches of a style sheet, in order.
pub fn segments(chars: &[char]) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let mut parens = 0usize;
    let mut push = |start: usize, end: usize, kind| {
        let mut a = start;
        let mut b = end;
        while a < b && chars[a].is_whitespace() {
            a += 1;
        }
        while b > a && chars[b - 1].is_whitespace() {
            b -= 1;
        }
        if a < b {
            out.push(Segment { kind, start: a, end: b });
        }
    };
    while i < chars.len() {
        if let Some(next) = skip_opaque(chars, i) {
            // A comment before a stretch isn't part of it.
            if chars[i] == '/' && chars[start..i].iter().all(|c| c.is_whitespace()) {
                start = next;
            }
            i = next;
            continue;
        }
        match chars[i] {
            '(' => parens += 1,
            ')' => parens = parens.saturating_sub(1),
            '{' if parens == 0 => {
                push(start, i, SegmentKind::Prelude);
                start = i + 1;
            }
            '}' | ';' if parens == 0 => {
                push(start, i, SegmentKind::Declaration);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    push(start, chars.len(), SegmentKind::Declaration);
    out
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_'
}

fn ident_end(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && (is_ident(chars[i]) || chars[i] == '\\') {
        i += if chars[i] == '\\' { 2 } else { 1 };
    }
    i.min(chars.len())
}

/// Index past the parenthesis opening at `i`.
fn after_parens(chars: &[char], i: usize) -> usize {
    let mut depth = 0;
    let mut j = i;
    while j < chars.len() {
        match chars[j] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return j + 1;
                }
            }
            _ => {}
        }
        j += 1;
    }
    j
}

/// Splits on commas outside parentheses and brackets, as (start, end) offsets.
fn split_list(chars: &[char], start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let (mut from, mut depth) = (start, 0i32);
    for (i, c) in chars.iter().enumerate().take(end).skip(start) {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push((from, i));
                from = i + 1;
            }
            _ => {}
        }
    }
    out.push((from, end));
    out
}

/// The most specific selector in a list.
fn max_of_list(chars: &[char]) -> (u32, u32, u32) {
    split_list(chars, 0, chars.len()).into_iter().map(|(a, b)| specificity_of(&chars[a..b])).max().unwrap_or((0, 0, 0))
}

fn specificity_of(sel: &[char]) -> (u32, u32, u32) {
    let (mut a, mut b, mut c) = (0, 0, 0);
    let mut add = |(x, y, z): (u32, u32, u32)| {
        a += x;
        b += y;
        c += z;
    };
    let mut i = 0;
    while i < sel.len() {
        match sel[i] {
            '#' => {
                add((1, 0, 0));
                i = ident_end(sel, i + 1);
            }
            '.' => {
                add((0, 1, 0));
                i = ident_end(sel, i + 1);
            }
            '[' => {
                add((0, 1, 0));
                while i < sel.len() && sel[i] != ']' {
                    i += 1;
                }
                i += 1;
            }
            ':' => {
                let element = sel.get(i + 1) == Some(&':');
                let from = if element { i + 2 } else { i + 1 };
                let end = ident_end(sel, from);
                let name: String = sel[from..end].iter().collect::<String>().to_lowercase();
                let args = (sel.get(end) == Some(&'(')).then(|| (end + 1, after_parens(sel, end)));
                let inner = args.map_or(&[][..], |(x, y)| &sel[x..y.saturating_sub(1).max(x)]);
                match name.as_str() {
                    _ if element => add((0, 0, 1)),
                    "before" | "after" | "first-line" | "first-letter" => add((0, 0, 1)),
                    "where" => {}
                    "is" | "not" | "has" | "matches" | "-webkit-any" | "-moz-any" => add(max_of_list(inner)),
                    "nth-child" | "nth-last-child" => {
                        add((0, 1, 0));
                        let text: String = inner.iter().collect();
                        if let Some(at) = text.find(" of ") {
                            let of: Vec<char> = text[at + 4..].chars().collect();
                            add(max_of_list(&of));
                        }
                    }
                    _ => add((0, 1, 0)),
                }
                i = args.map_or(end, |(_, y)| y);
            }
            ch if ch.is_alphabetic() || ch == '_' || ch == '-' || ch == '\\' => {
                add((0, 0, 1));
                i = ident_end(sel, i);
            }
            _ => i += 1,
        }
    }
    (a, b, c)
}

/// How specific a selector is: ids, then classes, then elements. A `&` counts
/// for nothing, because what it stands for is in another rule.
pub fn specificity(selector: &str) -> (u32, u32, u32) {
    specificity_of(&selector.chars().collect::<Vec<_>>())
}

/// The selector of a list under char `pos`, in a rule's prelude.
pub fn selector_at(text: &str, pos: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let segment = segments(&chars).into_iter().find(|s| s.start <= pos && pos <= s.end)?;
    if segment.kind != SegmentKind::Prelude || chars[segment.start] == '@' {
        return None;
    }
    let (a, b) = split_list(&chars, segment.start, segment.end).into_iter().find(|(a, b)| *a <= pos && pos <= *b)?;
    let selector: String = chars[a..b].iter().collect();
    Some(selector.split_whitespace().collect::<Vec<_>>().join(" "))
}

// ----- colors ----------------------------------------------------------------------

/// How a color is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Hex,
    /// The flag: with commas, the older way.
    Rgb(bool),
    Hsl(bool),
    Hwb,
    Lab,
    Lch,
    Oklab,
    Oklch,
    DisplayP3,
    SrgbFunction,
}

/// The smallest of the usual gamuts that holds a color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gamut {
    Srgb,
    DisplayP3,
    Wider,
}

/// A color, as extended sRGB: channels outside 0..=1 are outside the gamut.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub rgb: [f64; 3],
    pub alpha: f64,
    pub format: Format,
}

/// A color in a text, by char offsets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FoundColor {
    pub start: usize,
    pub end: usize,
    pub color: Color,
}

type M3 = [[f64; 3]; 3];

fn mul(m: &M3, v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

// From CSS Color 4's sample code.
const LINEAR_P3_TO_XYZ: M3 = [
    [0.486_570_948_6, 0.265_667_693_2, 0.198_217_285_2],
    [0.228_974_564_1, 0.691_738_521_8, 0.079_286_914_1],
    [0.0, 0.045_113_381_9, 1.043_944_368_9],
];
const XYZ_TO_LINEAR_P3: M3 = [
    [2.493_496_911_9, -0.931_383_617_9, -0.402_710_784_5],
    [-0.829_488_969_6, 1.762_664_060_3, 0.023_624_685_8],
    [0.035_845_830_2, -0.076_172_389_3, 0.956_884_524_0],
];
const LINEAR_SRGB_TO_XYZ: M3 = [
    [0.412_390_799_3, 0.357_584_339_4, 0.180_480_788_4],
    [0.212_639_005_9, 0.715_168_678_8, 0.072_192_315_4],
    [0.019_330_818_7, 0.119_194_779_8, 0.950_532_152_2],
];
const XYZ_TO_LINEAR_SRGB: M3 = [
    [3.240_969_941_9, -1.537_383_177_6, -0.498_610_760_3],
    [-0.969_243_636_3, 1.875_967_501_5, 0.041_555_057_4],
    [0.055_630_079_7, -0.203_976_958_9, 1.056_971_514_2],
];
const D50_TO_D65: M3 = [
    [0.955_473_452_7, -0.023_098_536_9, 0.063_259_308_7],
    [-0.028_369_707_0, 1.009_995_458_0, 0.021_041_399_0],
    [0.012_314_001_7, -0.020_507_696_4, 1.330_365_936_6],
];
const D65_TO_D50: M3 = [
    [1.047_929_792_5, 0.022_946_870_6, -0.050_192_266_3],
    [0.029_627_808_8, 0.990_434_426_8, -0.017_073_799_1],
    [-0.009_243_040_6, 0.015_055_191_5, 0.751_874_281_4],
];
const D50_WHITE: [f64; 3] = [0.964_22, 1.0, 0.825_21];

/// sRGB's transfer curve, extended to values outside 0..=1.
fn encode(x: f64) -> f64 {
    let a = x.abs();
    let y = if a <= 0.003_130_8 { 12.92 * a } else { 1.055 * a.powf(1.0 / 2.4) - 0.055 };
    y.copysign(x)
}

fn decode(x: f64) -> f64 {
    let a = x.abs();
    let y = if a <= 0.040_45 { a / 12.92 } else { ((a + 0.055) / 1.055).powf(2.4) };
    y.copysign(x)
}

fn from_linear(l: [f64; 3]) -> [f64; 3] {
    l.map(encode)
}

fn to_linear(rgb: [f64; 3]) -> [f64; 3] {
    rgb.map(decode)
}

fn oklab_to_linear([l, a, b]: [f64; 3]) -> [f64; 3] {
    let x = (l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let y = (l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let z = (l - 0.089_484_177_5 * a - 1.291_485_548_0 * b).powi(3);
    [
        4.076_741_662_1 * x - 3.307_711_591_3 * y + 0.230_969_929_2 * z,
        -1.268_438_004_6 * x + 2.609_757_401_1 * y - 0.341_319_396_5 * z,
        -0.004_196_086_3 * x - 0.703_418_614_7 * y + 1.707_614_701_0 * z,
    ]
}

fn linear_to_oklab([r, g, b]: [f64; 3]) -> [f64; 3] {
    let x = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let y = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let z = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    [
        0.210_454_255_3 * x + 0.793_617_785_0 * y - 0.004_072_046_8 * z,
        1.977_998_495_1 * x - 2.428_592_205_0 * y + 0.450_593_709_9 * z,
        0.025_904_037_1 * x + 0.782_771_766_2 * y - 0.808_675_766_0 * z,
    ]
}

fn lab_to_linear([l, a, b]: [f64; 3]) -> [f64; 3] {
    const KAPPA: f64 = 24389.0 / 27.0;
    const EPSILON: f64 = 216.0 / 24389.0;
    let f1 = (l + 16.0) / 116.0;
    let (f0, f2) = (a / 500.0 + f1, f1 - b / 200.0);
    let cube = |f: f64| if f.powi(3) > EPSILON { f.powi(3) } else { (116.0 * f - 16.0) / KAPPA };
    let y = if l > KAPPA * EPSILON { f1.powi(3) } else { l / KAPPA };
    let xyz = [cube(f0) * D50_WHITE[0], y, cube(f2) * D50_WHITE[2]];
    mul(&XYZ_TO_LINEAR_SRGB, mul(&D50_TO_D65, xyz))
}

fn linear_to_lab(linear: [f64; 3]) -> [f64; 3] {
    const KAPPA: f64 = 24389.0 / 27.0;
    const EPSILON: f64 = 216.0 / 24389.0;
    let xyz = mul(&D65_TO_D50, mul(&LINEAR_SRGB_TO_XYZ, linear));
    let f = |i: usize| {
        let v = xyz[i] / D50_WHITE[i];
        if v > EPSILON { v.cbrt() } else { (KAPPA * v + 16.0) / 116.0 }
    };
    [116.0 * f(1) - 16.0, 500.0 * (f(0) - f(1)), 200.0 * (f(1) - f(2))]
}

fn polar_to_rect([l, c, h]: [f64; 3]) -> [f64; 3] {
    let r = h.to_radians();
    [l, c * r.cos(), c * r.sin()]
}

fn rect_to_polar([l, a, b]: [f64; 3]) -> [f64; 3] {
    [l, a.hypot(b), b.atan2(a).to_degrees().rem_euclid(360.0)]
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [f64; 3] {
    let f = |n: f64| {
        let k = (n + h / 30.0).rem_euclid(12.0);
        l - s * l.min(1.0 - l) * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [f(0.0), f(8.0), f(4.0)]
}

fn rgb_to_hsl([r, g, b]: [f64; 3]) -> [f64; 3] {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let (d, l) = (max - min, (max + min) / 2.0);
    if d.abs() < 1e-9 {
        return [0.0, 0.0, l];
    }
    let s = if l <= 0.0 || l >= 1.0 { 0.0 } else { (max - l) / l.min(1.0 - l) };
    let h = if (max - r).abs() < 1e-12 {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < 1e-12 {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    [h * 60.0, s, l]
}

/// A number, a percentage of `full`, or `none`.
fn number(token: &str, full: f64) -> Option<f64> {
    if token.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    match token.strip_suffix('%') {
        Some(p) => p.parse::<f64>().ok().map(|v| v / 100.0 * full),
        None => token.parse().ok(),
    }
}

fn hue(token: &str) -> Option<f64> {
    if token.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    if let Some(v) = token.strip_suffix("deg") {
        return v.parse().ok();
    }
    if let Some(v) = token.strip_suffix("grad") {
        return v.parse::<f64>().ok().map(|g| g * 0.9);
    }
    if let Some(v) = token.strip_suffix("rad") {
        return v.parse::<f64>().ok().map(f64::to_degrees);
    }
    if let Some(v) = token.strip_suffix("turn") {
        return v.parse::<f64>().ok().map(|t| t * 360.0);
    }
    token.parse().ok()
}

/// A function's arguments: three channels and an alpha.
fn function_color(name: &str, args: &str) -> Option<Color> {
    let commas = args.contains(',');
    let tokens: Vec<&str> =
        args.split(|c: char| c.is_whitespace() || c == ',' || c == '/').filter(|t| !t.is_empty()).collect();
    let (space, tokens) = if name == "color" { (Some(*tokens.first()?), &tokens[1..]) } else { (None, &tokens[..]) };
    if tokens.len() != 3 && tokens.len() != 4 {
        return None;
    }
    let alpha = tokens.get(3).map_or(Some(1.0), |t| number(t, 1.0))?.clamp(0.0, 1.0);
    let (t0, t1, t2) = (tokens[0], tokens[1], tokens[2]);
    let (rgb, format) = match (name, space) {
        ("rgb" | "rgba", _) => {
            let c = [number(t0, 255.0)?, number(t1, 255.0)?, number(t2, 255.0)?];
            (c.map(|v| v / 255.0), Format::Rgb(commas))
        }
        ("hsl" | "hsla", _) => (hsl_to_rgb(hue(t0)?, number(t1, 1.0)?, number(t2, 1.0)?), Format::Hsl(commas)),
        ("hwb", _) => {
            let (w, b) = (number(t1, 1.0)?, number(t2, 1.0)?);
            let rgb = if w + b >= 1.0 {
                [w / (w + b); 3]
            } else {
                hsl_to_rgb(hue(t0)?, 1.0, 0.5).map(|v| v * (1.0 - w - b) + w)
            };
            (rgb, Format::Hwb)
        }
        ("lab", _) => {
            let lab = [number(t0, 100.0)?, number(t1, 125.0)?, number(t2, 125.0)?];
            (from_linear(lab_to_linear(lab)), Format::Lab)
        }
        ("lch", _) => {
            let lab = polar_to_rect([number(t0, 100.0)?, number(t1, 150.0)?, hue(t2)?]);
            (from_linear(lab_to_linear(lab)), Format::Lch)
        }
        ("oklab", _) => {
            let lab = [number(t0, 1.0)?, number(t1, 0.4)?, number(t2, 0.4)?];
            (from_linear(oklab_to_linear(lab)), Format::Oklab)
        }
        ("oklch", _) => {
            let lab = polar_to_rect([number(t0, 1.0)?, number(t1, 0.4)?, hue(t2)?]);
            (from_linear(oklab_to_linear(lab)), Format::Oklch)
        }
        ("color", Some("display-p3")) => {
            let p3 = [number(t0, 1.0)?, number(t1, 1.0)?, number(t2, 1.0)?];
            let linear = mul(&XYZ_TO_LINEAR_SRGB, mul(&LINEAR_P3_TO_XYZ, to_linear(p3)));
            (from_linear(linear), Format::DisplayP3)
        }
        ("color", Some("srgb")) => ([number(t0, 1.0)?, number(t1, 1.0)?, number(t2, 1.0)?], Format::SrgbFunction),
        _ => return None,
    };
    Some(Color { rgb, alpha, format })
}

fn hex_color(digits: &str) -> Option<Color> {
    let n = digits.len();
    if !matches!(n, 3 | 4 | 6 | 8) || !digits.is_ascii() {
        return None;
    }
    let byte = |i: usize| -> Option<f64> {
        let v = if n <= 4 {
            u8::from_str_radix(&digits[i..=i], 16).ok()? * 17
        } else {
            u8::from_str_radix(&digits[i * 2..i * 2 + 2], 16).ok()?
        };
        Some(f64::from(v) / 255.0)
    };
    let alpha = if n == 4 || n == 8 { byte(3)? } else { 1.0 };
    Some(Color { rgb: [byte(0)?, byte(1)?, byte(2)?], alpha, format: Format::Hex })
}

const COLOR_FUNCTIONS: [&str; 11] =
    ["rgb", "rgba", "hsl", "hsla", "hwb", "lab", "lch", "oklab", "oklch", "color", "display-p3"];

/// Every color written as a hex value or a function. Named colors aren't read.
pub fn colors(text: &str) -> Vec<FoundColor> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '/'
            && let Some(next) = skip_opaque(&chars, i)
        {
            i = next;
        } else if chars[i] == '#' {
            let end = ident_end(&chars, i + 1);
            let digits: String = chars[i + 1..end].iter().collect();
            if let Some(color) = hex_color(&digits) {
                out.push(FoundColor { start: i, end, color });
            }
            i = end.max(i + 1);
        } else if chars[i].is_alphabetic() && (i == 0 || !is_ident(chars[i - 1])) {
            let end = ident_end(&chars, i);
            let name: String = chars[i..end].iter().collect::<String>().to_lowercase();
            if chars.get(end) == Some(&'(') && COLOR_FUNCTIONS.contains(&name.as_str()) {
                let close = after_parens(&chars, end);
                let args: String = chars[end + 1..close.saturating_sub(1).max(end + 1)].iter().collect();
                if let Some(color) = function_color(&name, &args) {
                    out.push(FoundColor { start: i, end: close, color });
                    i = close;
                    continue;
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// The color under char `pos`.
pub fn color_at(text: &str, pos: usize) -> Option<FoundColor> {
    colors(text).into_iter().find(|f| f.start <= pos && pos <= f.end)
}

/// In the rule around `pos`, the color the one there is seen against: the
/// background for a text color, the text color for a background.
pub fn counterpart(text: &str, pos: usize) -> Option<Color> {
    let chars: Vec<char> = text.chars().collect();
    let segments = segments(&chars);
    let here = segments.iter().position(|s| s.start <= pos && pos <= s.end)?;
    let property = |s: &Segment| -> String {
        chars[s.start..s.end].iter().take_while(|c| **c != ':').collect::<String>().trim().to_lowercase()
    };
    let is_background = property(&segments[here]).starts_with("background");
    let wanted = |name: &str| if is_background { name == "color" } else { name.starts_with("background") };
    let block = |step: fn(usize) -> Option<usize>| {
        let mut i = here;
        std::iter::from_fn(move || {
            i = step(i)?;
            Some(i)
        })
        .map_while(|i| segments.get(i).filter(|s| s.kind == SegmentKind::Declaration))
        .find(|s| wanted(&property(s)))
    };
    let found = block(|i| i.checked_sub(1)).or_else(|| block(|i| Some(i + 1)))?;
    let value: String = chars[found.start..found.end].iter().collect();
    colors(&value).first().map(|f| f.color)
}

impl Color {
    pub fn from_srgb8(r: u8, g: u8, b: u8, a: u8, format: Format) -> Self {
        let f = |v: u8| f64::from(v) / 255.0;
        Self { rgb: [f(r), f(g), f(b)], alpha: f(a), format }
    }

    /// Channels clamped into sRGB, as bytes.
    pub fn srgb8(&self) -> [u8; 4] {
        let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [byte(self.rgb[0]), byte(self.rgb[1]), byte(self.rgb[2]), byte(self.alpha)]
    }

    pub fn gamut(&self) -> Gamut {
        const SLACK: f64 = 0.000_5;
        let fits = |v: &[f64; 3]| v.iter().all(|c| (-SLACK..=1.0 + SLACK).contains(c));
        if fits(&self.rgb) {
            return Gamut::Srgb;
        }
        let p3 = from_linear(mul(&XYZ_TO_LINEAR_P3, mul(&LINEAR_SRGB_TO_XYZ, to_linear(self.rgb))));
        if fits(&p3) { Gamut::DisplayP3 } else { Gamut::Wider }
    }

    /// Lightness, chroma and hue.
    pub fn oklch(&self) -> [f64; 3] {
        rect_to_polar(linear_to_oklab(to_linear(self.rgb)))
    }

    pub fn from_oklch(lch: [f64; 3], alpha: f64, format: Format) -> Self {
        Self { rgb: from_linear(oklab_to_linear(polar_to_rect(lch))), alpha, format }
    }

    /// Relative luminance after clamping into sRGB, as WCAG defines it.
    fn luminance(rgb: [f64; 3]) -> f64 {
        let [r, g, b] = rgb.map(|v| decode(v.clamp(0.0, 1.0)));
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// WCAG's contrast ratio, 1 to 21, with this color laid over an opaque `background`.
    pub fn contrast(&self, background: &Color) -> f64 {
        let over = |i: usize| self.rgb[i] * self.alpha + background.rgb[i] * (1.0 - self.alpha);
        let (a, b) = (Self::luminance([over(0), over(1), over(2)]), Self::luminance(background.rgb));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// The color as CSS, in its format.
    pub fn to_css(&self) -> String {
        let n = |v: f64, places: usize| {
            let text = format!("{v:.places$}");
            let text = if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.') } else { &text };
            if text == "-0" { "0".to_owned() } else { text.to_owned() }
        };
        let [r, g, b, a] = self.srgb8();
        let opaque = self.alpha >= 0.999;
        let slash = if opaque { String::new() } else { format!(" / {}", n(self.alpha, 3)) };
        let linear = to_linear(self.rgb);
        match self.format {
            Format::Hex if opaque => format!("#{r:02x}{g:02x}{b:02x}"),
            Format::Hex => format!("#{r:02x}{g:02x}{b:02x}{a:02x}"),
            Format::Rgb(true) if opaque => format!("rgb({r}, {g}, {b})"),
            Format::Rgb(true) => format!("rgba({r}, {g}, {b}, {})", n(self.alpha, 3)),
            Format::Rgb(false) => format!("rgb({r} {g} {b}{slash})"),
            Format::Hsl(commas) => {
                let [h, s, l] = rgb_to_hsl(self.rgb);
                let (h, s, l) = (n(h, 1), n(s * 100.0, 1), n(l * 100.0, 1));
                match (commas, opaque) {
                    (true, true) => format!("hsl({h}, {s}%, {l}%)"),
                    (true, false) => format!("hsla({h}, {s}%, {l}%, {})", n(self.alpha, 3)),
                    (false, _) => format!("hsl({h} {s}% {l}%{slash})"),
                }
            }
            Format::Hwb => {
                let [h, ..] = rgb_to_hsl(self.rgb);
                let (max, min) =
                    (self.rgb[0].max(self.rgb[1]).max(self.rgb[2]), self.rgb[0].min(self.rgb[1]).min(self.rgb[2]));
                format!("hwb({} {}% {}%{slash})", n(h, 1), n(min * 100.0, 1), n((1.0 - max) * 100.0, 1))
            }
            Format::Lab => {
                let [l, x, y] = linear_to_lab(linear);
                format!("lab({}% {} {}{slash})", n(l, 2), n(x, 2), n(y, 2))
            }
            Format::Lch => {
                let [l, c, h] = rect_to_polar(linear_to_lab(linear));
                format!("lch({}% {} {}{slash})", n(l, 2), n(c, 2), n(h, 1))
            }
            Format::Oklab => {
                let [l, x, y] = linear_to_oklab(linear);
                format!("oklab({} {} {}{slash})", n(l, 4), n(x, 4), n(y, 4))
            }
            Format::Oklch => {
                let [l, c, h] = self.oklch();
                format!("oklch({} {} {}{slash})", n(l, 4), n(c, 4), n(h, 2))
            }
            Format::DisplayP3 => {
                let [x, y, z] = from_linear(mul(&XYZ_TO_LINEAR_P3, mul(&LINEAR_SRGB_TO_XYZ, linear)));
                format!("color(display-p3 {} {} {}{slash})", n(x, 4), n(y, 4), n(z, 4))
            }
            Format::SrgbFunction => {
                let [x, y, z] = self.rgb;
                format!("color(srgb {} {} {}{slash})", n(x, 4), n(y, 4), n(z, 4))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 0.002)
    }

    #[test]
    fn specificity_follows_the_spec() {
        // Examples from Selectors Level 4 and MDN.
        assert_eq!(specificity("*"), (0, 0, 0));
        assert_eq!(specificity("li"), (0, 0, 1));
        assert_eq!(specificity("ul li"), (0, 0, 2));
        assert_eq!(specificity("ul ol+li"), (0, 0, 3));
        assert_eq!(specificity("h1 + *[rel=up]"), (0, 1, 1));
        assert_eq!(specificity("ul ol li.red"), (0, 1, 3));
        assert_eq!(specificity("li.red.level"), (0, 2, 1));
        assert_eq!(specificity("#x34y"), (1, 0, 0));
        assert_eq!(specificity("#s12:not(FOO)"), (1, 0, 1));
        assert_eq!(specificity(".foo :is(.bar, #baz)"), (1, 1, 0));
        assert_eq!(specificity(":where(#a, .b) p"), (0, 0, 1));
        assert_eq!(specificity("a:hover::before"), (0, 1, 2));
        assert_eq!(specificity("a:after"), (0, 0, 2));
        assert_eq!(specificity(":nth-child(2n+1 of .a, #b)"), (1, 1, 0));
        assert_eq!(specificity("& .x"), (0, 1, 0));
    }

    #[test]
    fn finds_the_selector_under_the_pointer() {
        let text = "/* a, b */\nul li.x, #y > p {\n  color: red;\n}\n@media (x) { a { b: c } }\n";
        let at = |needle: &str| text.find(needle).unwrap();
        assert_eq!(selector_at(text, at("li.x")).as_deref(), Some("ul li.x"));
        assert_eq!(selector_at(text, at("#y")).as_deref(), Some("#y > p"));
        assert_eq!(selector_at(text, at("color")), None);
        assert_eq!(selector_at(text, at("@media")), None);
        assert_eq!(selector_at(text, at("a {")).as_deref(), Some("a"));
    }

    #[test]
    fn reads_hex_and_functions_and_keeps_their_format() {
        let one = |text: &str| colors(text).first().copied().unwrap().color;
        for text in [
            "#ff8000",
            "#ff800080",
            "rgb(255, 128, 0)",
            "rgb(255 128 0)",
            "rgb(255 128 0 / 0.5)",
            "rgba(255, 128, 0, 0.5)",
            "hsl(30, 100%, 50%)",
            "hsl(30 100% 50%)",
        ] {
            assert_eq!(one(text).to_css(), text, "{text}");
        }
        assert_eq!(one("#F80").to_css(), "#ff8800");
        assert_eq!(one("#f80").srgb8(), [255, 136, 0, 255]);
        assert!(close(one("hwb(0 0% 0%)").rgb, [1.0, 0.0, 0.0]));
        assert_eq!(one("hwb(120 20% 30%)").to_css(), "hwb(120 20% 30%)");
    }

    #[test]
    fn wide_gamut_functions_match_known_values() {
        let rgb = |text: &str| colors(text).first().unwrap().color.rgb;
        // CSS Color 4's own values for sRGB red.
        assert!(close(rgb("oklch(0.62796 0.25768 29.234)"), [1.0, 0.0, 0.0]));
        assert!(close(rgb("oklab(0.62796 0.22486 0.12585)"), [1.0, 0.0, 0.0]));
        assert!(close(rgb("lab(54.29 80.82 69.91)"), [1.0, 0.0, 0.0]));
        assert!(close(rgb("lch(54.29 106.84 40.85)"), [1.0, 0.0, 0.0]));
        assert!(close(rgb("color(srgb 1 0 0)"), [1.0, 0.0, 0.0]));
        assert!(close(rgb("color(display-p3 1 1 1)"), [1.0, 1.0, 1.0]));
    }

    #[test]
    fn formats_round_trip() {
        for text in [
            "oklch(0.7 0.1 200)",
            "oklab(0.7 0.05 -0.1)",
            "lab(60 20 -30)",
            "lch(60 40 200)",
            "color(display-p3 0.5 0.2 0.8)",
            "hsl(200 50% 40%)",
        ] {
            let first = colors(text)[0].color;
            let again = colors(&first.to_css())[0].color;
            assert!(close(first.rgb, again.rgb), "{text}: {}", first.to_css());
            assert_eq!(first.format, again.format);
        }
    }

    #[test]
    fn tells_the_gamut() {
        let gamut = |text: &str| colors(text)[0].color.gamut();
        assert_eq!(gamut("#ff0000"), Gamut::Srgb);
        assert_eq!(gamut("oklch(0.62796 0.25768 29.234)"), Gamut::Srgb);
        assert_eq!(gamut("color(display-p3 1 0 0)"), Gamut::DisplayP3);
        assert_eq!(gamut("oklch(0.7 0.4 150)"), Gamut::Wider);
    }

    #[test]
    fn contrast_matches_wcag() {
        let color = |text: &str| colors(text)[0].color;
        assert!((color("#000").contrast(&color("#fff")) - 21.0).abs() < 0.01);
        assert!((color("#777").contrast(&color("#fff")) - 4.48).abs() < 0.01);
        assert!((color("#fff").contrast(&color("#fff")) - 1.0).abs() < 0.01);
        // Half-transparent black on white is mid grey.
        let grey = color("#00000080").contrast(&color("#fff"));
        assert!((3.9..4.1).contains(&grey), "{grey}");
    }

    #[test]
    fn ignores_what_only_looks_like_a_color() {
        assert!(colors("#main { x: var(--c); y: rgb(var(--r) 0 0); }").is_empty());
        assert!(colors("/* #fff */ .a { }").is_empty());
        assert!(colors("a { width: calc(1px); }").is_empty());
    }

    #[test]
    fn finds_what_a_color_is_seen_against() {
        let text = ".a { background: #000; color: #fff; }\n.b { color: #111; }\n";
        let at = |needle: &str| text.find(needle).unwrap() + 1;
        assert_eq!(counterpart(text, at("#fff")).unwrap().srgb8(), [0, 0, 0, 255]);
        assert_eq!(counterpart(text, at("#000")).unwrap().srgb8(), [255, 255, 255, 255]);
        assert_eq!(counterpart(text, at("#111")), None);
    }
}
