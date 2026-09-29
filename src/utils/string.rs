//! Port of `libSiON-cpp/src/compat/sion_string.h` (Godot `String` compat
//! helpers) as free functions over `&str`/`String`, plus the global
//! `itos`/`rtos` helpers from the same header. The `sion::String` class is
//! backed by bytes (ASCII MML payloads), so `usize` byte offsets replace the
//! C++ `size_t` positions. Also contains the `RegEx::sub` literal-replacement
//! helpers documented in `docs/CONVENTIONS.md` (ported from
//! `src/compat/sion_regex.cpp`), and `split_string_by_regex` ported from
//! `src/utils/godot_util.cpp`.

use regex::Regex;

// --- Introspection.

/// `String::length()` (signed version).
pub fn length(s: &str) -> i64 {
    s.len() as i64
}

/// `String::is_empty()`.
pub fn is_empty(s: &str) -> bool {
    s.is_empty()
}

/// `String::unicode_at()` — byte at `p_pos` as a codepoint, 0 when out of range.
pub fn unicode_at(s: &str, p_pos: usize) -> i32 {
    match s.as_bytes().get(p_pos) {
        Some(b) => i32::from(*b),
        None => 0,
    }
}

/// `String::begins_with()`.
pub fn begins_with(s: &str, p_str: &str) -> bool {
    s.len() >= p_str.len() && s.as_bytes()[..p_str.len()] == p_str.as_bytes()[..]
}

/// `String::ends_with()`.
pub fn ends_with(s: &str, p_str: &str) -> bool {
    s.len() >= p_str.len() && s.as_bytes()[s.len() - p_str.len()..] == p_str.as_bytes()[..]
}

/// `String::contains()`.
pub fn contains(s: &str, p_str: &str) -> bool {
    s.contains(p_str)
}

// --- Conversions.

/// True when the string starts (after spaces/tabs and an optional sign) with a
/// `0x`/`0X` hex prefix — `String::_is_hex_prefixed()`.
fn is_hex_prefixed(bytes: &[u8]) -> bool {
    let mut i = 0;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        i += 1;
    }
    i + 1 < bytes.len() && bytes[i] == b'0' && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X')
}

/// One `strtoll` digit in `p_base` (10 or 16), else `None`.
fn digit_value(b: u8, p_base: u32) -> Option<u64> {
    let v = match b {
        b'0'..=b'9' => u64::from(b - b'0'),
        b'a'..=b'f' => u64::from(b - b'a') + 10,
        b'A'..=b'F' => u64::from(b - b'A') + 10,
        _ => return None,
    };
    if v < u64::from(p_base) { Some(v) } else { None }
}

/// `strtoll` parity: skip leading whitespace (isspace set) and sign, then
/// parse digits in `p_base`, stopping at the first non-digit; saturates on
/// overflow like C. Returns 0 when no digits parse.
fn parse_ll(s: &str, p_base: u32) -> i64 {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len()
        && matches!(bytes[i], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
    {
        i += 1;
    }
    let mut negative = false;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        negative = bytes[i] == b'-';
        i += 1;
    }
    if p_base == 16 && i + 1 < bytes.len() && bytes[i] == b'0'
        && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X')
    {
        i += 2;
    }
    let mut acc: u64 = 0;
    let mut any = false;
    let mut overflow = false;
    while i < bytes.len() {
        match digit_value(bytes[i], p_base) {
            Some(d) => {
                any = true;
                acc = match acc
                    .checked_mul(u64::from(p_base))
                    .and_then(|v| v.checked_add(d))
                {
                    Some(v) => v,
                    None => {
                        overflow = true;
                        u64::MAX
                    }
                };
                i += 1;
            }
            None => break,
        }
    }
    if !any {
        return 0;
    }
    if negative {
        if overflow || acc > (i64::MAX as u64) + 1 {
            i64::MIN
        } else if acc == (i64::MAX as u64) + 1 {
            // -9223372036854775808 literal — wraps to i64::MIN.
            i64::MIN
        } else {
            -(acc as i64)
        }
    } else if overflow || acc > i64::MAX as u64 {
        i64::MAX
    } else {
        acc as i64
    }
}

/// `String::to_int()` — Godot semantics: parse a leading number and ignore
/// the rest; `0x`-prefixed literals are parsed as hex.
pub fn to_int(s: &str) -> i64 {
    parse_ll(s, if is_hex_prefixed(s.as_bytes()) { 16 } else { 10 })
}

/// `String::hex_to_int()` — `strtoll(s, nullptr, 16)`.
pub fn hex_to_int(s: &str) -> i64 {
    parse_ll(s, 16)
}

/// `String::to_float()` — `strtod` parity for the decimal forms MML uses.
pub fn to_float(s: &str) -> f64 {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len()
        && matches!(bytes[i], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
    {
        i += 1;
    }
    let start = i;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        i += 1;
    }
    let mut digits = false;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        digits = true;
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            digits = true;
            i += 1;
        }
    }
    if !digits {
        return 0.0;
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        let save = i;
        i += 1;
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        let mut exp_digits = false;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            exp_digits = true;
            i += 1;
        }
        if !exp_digits {
            i = save; // Exponent absent — strtod stops before the 'e'.
        }
    }
    s[start..i].parse::<f64>().unwrap_or(0.0)
}

/// `String::is_valid_int()`.
pub fn is_valid_int(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut c = 0;
    if c < bytes.len() && (bytes[c] == b'+' || bytes[c] == b'-') {
        c += 1;
    }
    let mut any = false;
    while c < bytes.len() {
        if !bytes[c].is_ascii_digit() {
            return false;
        }
        any = true;
        c += 1;
    }
    any
}

/// `String::is_valid_float()`.
pub fn is_valid_float(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut c = 0;
    if c < bytes.len() && (bytes[c] == b'+' || bytes[c] == b'-') {
        c += 1;
    }
    let mut digits_before = false;
    let mut digits_after = false;
    while c < bytes.len() && bytes[c] >= b'0' && bytes[c] <= b'9' {
        digits_before = true;
        c += 1;
    }
    if c < bytes.len() && bytes[c] == b'.' {
        c += 1;
        while c < bytes.len() && bytes[c] >= b'0' && bytes[c] <= b'9' {
            digits_after = true;
            c += 1;
        }
    }
    if !digits_before && !digits_after {
        return false;
    }
    if c < bytes.len() && (bytes[c] == b'e' || bytes[c] == b'E') {
        c += 1;
        if c < bytes.len() && (bytes[c] == b'+' || bytes[c] == b'-') {
            c += 1;
        }
        let mut digits_exp = false;
        while c < bytes.len() && bytes[c] >= b'0' && bytes[c] <= b'9' {
            digits_exp = true;
            c += 1;
        }
        if !digits_exp {
            return false;
        }
    }
    c == bytes.len()
}

// --- Searching. Godot semantics: -1 when absent.

/// `String::find(const String &, size_t)` — returns -1 when absent.
pub fn find(s: &str, p_str: &str, p_from: usize) -> i64 {
    if p_from > s.len() {
        return -1;
    }
    match s[p_from..].find(p_str) {
        Some(r) => (r + p_from) as i64,
        None => -1,
    }
}

/// `String::find(char, size_t)` — returns -1 when absent.
pub fn find_char(s: &str, p_char: u8, p_from: usize) -> i64 {
    if p_from > s.len() {
        return -1;
    }
    match s.as_bytes()[p_from..].iter().position(|b| *b == p_char) {
        Some(r) => (r + p_from) as i64,
        None => -1,
    }
}

// --- Slicing. Godot semantics: never throws, clamps instead.

/// `String::substr(p_from, p_len)` — empty when `p_from > length`.
pub fn substr(s: &str, p_from: usize, p_len: usize) -> String {
    if p_from > s.len() {
        return String::new();
    }
    let end = p_from.saturating_add(p_len).min(s.len());
    s[p_from..end].to_string()
}

/// `String::substr(p_from)` (to the end).
pub fn substr_to_end(s: &str, p_from: usize) -> String {
    substr(s, p_from, usize::MAX)
}

/// `String::left(p_count)`.
pub fn left(s: &str, p_count: usize) -> String {
    substr(s, 0, p_count)
}

/// `String::right(p_count)`.
pub fn right(s: &str, p_count: usize) -> String {
    if p_count >= s.len() {
        s.to_string()
    } else {
        substr_to_end(s, s.len() - p_count)
    }
}

// --- Mutation helpers (Godot style). insert/replace are non-mutating and
// return the new string; erase mutates in place.

/// `String::insert(p_pos, p_str)` — non-mutating.
pub fn insert(s: &str, p_pos: usize, p_str: &str) -> String {
    let pos = p_pos.min(s.len());
    let mut result = String::with_capacity(s.len() + p_str.len());
    result.push_str(&s[..pos]);
    result.push_str(p_str);
    result.push_str(&s[pos..]);
    result
}

/// `String::erase(p_pos, p_len)` — mutating, deleting `p_len` bytes (1 by
/// default).
pub fn erase(s: &mut String, p_pos: usize, p_len: usize) {
    if p_pos >= s.len() {
        return;
    }
    let end = p_pos.saturating_add(p_len).min(s.len());
    s.replace_range(p_pos..end, "");
}

/// `String::replace(p_what, p_forwhat)` — non-mutating, replaces all.
pub fn replace(s: &str, p_what: &str, p_forwhat: &str) -> String {
    if p_what.is_empty() {
        return s.to_string();
    }
    let mut result = String::new();
    let mut pos = 0usize;
    loop {
        match find(s, p_what, pos) {
            -1 => {
                result.push_str(&s[pos..]);
                break;
            }
            f => {
                let found = f as usize;
                result.push_str(&s[pos..found]);
                result.push_str(p_forwhat);
                pos = found + p_what.len();
            }
        }
    }
    result
}

/// `String::repeat(p_count)`.
pub fn repeat(s: &str, p_count: u64) -> String {
    let mut result = String::with_capacity(s.len().saturating_mul(p_count as usize));
    for _ in 0..p_count {
        result.push_str(s);
    }
    result
}

// --- Case conversion (ASCII-only, sufficient for MML identifiers).

/// `String::to_lower()`.
pub fn to_lower(s: &str) -> String {
    let mut bytes = s.as_bytes().to_vec();
    for c in &mut bytes {
        if c.is_ascii_uppercase() {
            *c += b'a' - b'A';
        }
    }
    String::from_utf8(bytes).unwrap_or_else(|_| s.to_string())
}

/// `String::to_upper()`.
pub fn to_upper(s: &str) -> String {
    let mut bytes = s.as_bytes().to_vec();
    for c in &mut bytes {
        if c.is_ascii_lowercase() {
            *c -= b'a' - b'A';
        }
    }
    String::from_utf8(bytes).unwrap_or_else(|_| s.to_string())
}

// --- Padding. Godot parity: grow to p_digits characters with leading zeros,
// keeping a leading sign in place.

/// `String::pad_zeros(p_digits)`.
pub fn pad_zeros(s: &str, p_digits: i32) -> String {
    let len = s.len() as i32;
    if len >= p_digits {
        return s.to_string();
    }
    let mut result = String::new();
    let mut offset = 0usize;
    if !s.is_empty() {
        let first = s.as_bytes()[0];
        if first == b'-' || first == b'+' {
            result.push(first as char);
            offset = 1;
        }
    }
    for _ in 0..(p_digits - len + offset as i32) {
        result.push('0');
    }
    result.push_str(&s[offset..]);
    result
}

// --- Splitting and joining. Godot semantics.

/// `String::split(p_delim, p_allow_empty, p_maxsplit)`.
pub fn split(s: &str, p_delim: &str, p_allow_empty: bool, p_maxsplit: usize) -> Vec<String> {
    let mut result = Vec::new();
    if s.is_empty() {
        return result;
    }
    if p_delim.is_empty() {
        result.push(s.to_string());
        return result;
    }

    let mut splits = 0usize;
    let mut pos = 0usize;
    loop {
        let found = find(s, p_delim, pos);
        if found == -1 || (p_maxsplit > 0 && splits >= p_maxsplit) {
            let piece = substr_to_end(s, pos);
            if p_allow_empty || !piece.is_empty() {
                result.push(piece);
            }
            break;
        }
        let found = found as usize;
        let piece = s[pos..found].to_string();
        if p_allow_empty || !piece.is_empty() {
            result.push(piece);
        }
        splits += 1;
        pos = found + p_delim.len();
    }
    result
}

/// `String::get_slice(p_delim, p_slice)`.
pub fn get_slice(s: &str, p_delim: &str, p_slice: i64) -> String {
    if p_delim.is_empty() || p_slice < 0 {
        return String::new();
    }

    let mut index = 0i64;
    let mut pos = 0usize;
    loop {
        let found = find(s, p_delim, pos);
        if found == -1 {
            return if index == p_slice { substr_to_end(s, pos) } else { String::new() };
        }
        let found = found as usize;
        if index == p_slice {
            return s[pos..found].to_string();
        }
        index += 1;
        pos = found + p_delim.len();
    }
}

/// `String::join(p_parts)` — `self` is the separator.
pub fn join(sep: &str, p_parts: &[String]) -> String {
    let mut result = String::new();
    for (i, part) in p_parts.iter().enumerate() {
        if i > 0 {
            result.push_str(sep);
        }
        result.push_str(part);
    }
    result
}

/// `String::strip_edges(p_left, p_right)`.
pub fn strip_edges(s: &str, p_left: bool, p_right: bool) -> String {
    let bytes = s.as_bytes();
    let mut begin = 0usize;
    let mut end = s.len();
    let is_edge = |b: u8| b == b' ' || b == b'\t' || b == b'\n' || b == b'\r';
    if p_left {
        while begin < end && is_edge(bytes[begin]) {
            begin += 1;
        }
    }
    if p_right {
        while end > begin && is_edge(bytes[end - 1]) {
            end -= 1;
        }
    }
    s[begin..end].to_string()
}

// --- Free helpers matching Godot's global itos/rtos.

/// Godot `itos(int64_t)`.
pub fn itos(p_value: i64) -> String {
    p_value.to_string()
}

/// Godot `rtos(double)` — `snprintf(buffer, sizeof(buffer), "%.5f", value)`.
pub fn rtos(p_value: f64) -> String {
    format!("{:.5}", p_value)
}

// --- RegEx::sub literal replacement (ported from `src/compat/sion_regex.cpp`).

/// `RegEx::sub(subject, replacement, count)` — literal replacement of up to
/// `p_count` matches (negative = all), matching the compat shim's empty-match
/// cursor handling exactly. No `$`-expansion of the replacement.
pub fn regex_sub(re: &Regex, subject: &str, replacement: &str, p_count: i64) -> String {
    if p_count == 0 {
        return subject.to_string();
    }

    let mut result = String::new();
    let mut offset = 0usize;
    let mut replaced = 0i64;

    for m in re.find_iter(subject) {
        result.push_str(&subject[offset..m.start()]);
        result.push_str(replacement);
        replaced += 1;

        if m.start() == m.end() {
            // Empty match: copy one character forward to avoid looping.
            if offset < subject.len() {
                result.push(subject.as_bytes()[offset] as char);
            }
            offset += 1;
        } else {
            offset = m.end();
        }

        if p_count >= 0 && replaced >= p_count {
            break;
        }
    }

    if offset <= subject.len() {
        result.push_str(&subject[offset.min(subject.len())..]);
    }
    result
}

/// `RegEx::sub(subject, replacement, p_all)` with `p_all = true` — replace ALL
/// matches literally (the SiON override semantics from `sion_regex.h`).
pub fn literal_replace_all(re: &Regex, subject: &str, replacement: &str) -> String {
    regex_sub(re, subject, replacement, -1)
}

/// `RegEx::sub(subject, replacement, 0)` — no-op returning the subject.
pub fn regex_sub_none(re: &Regex, subject: &str) -> String {
    regex_sub(re, subject, "", 0)
}

/// Port of `split_string_by_regex()` from `src/utils/godot_util.cpp`.
pub fn split_string_by_regex(p_string: &str, p_regex: &str) -> Vec<String> {
    // This is boilerplate to split a string by a regex in Godot.
    let mut arr: Vec<String> = Vec::new();

    let Ok(re_split) = Regex::new(p_regex) else {
        return arr;
    };
    let matches: Vec<(usize, usize)> = re_split
        .find_iter(p_string)
        .map(|m| (m.start(), m.end()))
        .collect();

    let mut last_index = 0usize;
    for (start, end) in matches {
        let piece = substr(p_string, last_index, start - last_index);
        arr.push(piece);
        last_index = end;
    }

    let last_match = substr_to_end(p_string, last_index);
    arr.push(last_match);

    arr
}
