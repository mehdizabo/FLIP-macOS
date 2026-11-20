//! Sensitive-data detection over OCR words ("Oscura" / REDACT): finds IBANs, cards, emails,
//! tax codes, VAT numbers and phone numbers, and returns the boxes that must be covered.

use serde::Deserialize;

/// One recognised word with its box.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OcrWord {
    /// Recognised text.
    pub text: String,
    /// Box `[x, y, w, h]`, normalised to 0..1 with a top-left origin.
    #[serde(rename = "box")]
    pub bbox: [f64; 4],
}

/// One OCR line with its words in reading order.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OcrLine {
    /// Words of the line.
    pub words: Vec<OcrWord>,
}

/// OCR result for one image.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OcrBoxes {
    /// Pixel width of the upright image.
    pub width: u32,
    /// Pixel height of the upright image.
    pub height: u32,
    /// Lines in reading order.
    pub lines: Vec<OcrLine>,
}

/// Kind of sensitive data found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensitiveKind {
    /// IBAN account number.
    Iban,
    /// Payment card number.
    Card,
    /// Email address.
    Email,
    /// Italian codice fiscale.
    TaxCode,
    /// Italian partita IVA.
    VatNumber,
    /// Phone number.
    Phone,
}

/// Box to cover, normalised to 0..1 with a top-left origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    /// Kind of data the box hides.
    pub kind: SensitiveKind,
    /// Normalised left edge.
    pub x: f64,
    /// Normalised top edge.
    pub y: f64,
    /// Normalised width.
    pub w: f64,
    /// Normalised height.
    pub h: f64,
}

/// Detector order and maximum window width: a word claimed by an earlier kind cannot be reused.
const DETECTORS: [(SensitiveKind, usize); 6] = [
    (SensitiveKind::Iban, 9),
    (SensitiveKind::Card, 5),
    (SensitiveKind::Email, 1),
    (SensitiveKind::TaxCode, 1),
    (SensitiveKind::VatNumber, 3),
    (SensitiveKind::Phone, 4),
];

/// Punctuation stripped from the start of a word before matching.
const LEADING_TRIM: &[char] = &[',', ';', ':', '(', ')', '[', ']', '<', '>', '"', '\''];
/// Punctuation stripped from the end of a word before matching (the trailing `.` included).
const TRAILING_TRIM: &[char] = &[',', ';', ':', '(', ')', '[', ']', '<', '>', '"', '\'', '.'];

/// Codice fiscale check-character values, indexed by digit 0-9 then letter A-Z, for characters
/// in odd 1-based positions of the first 15 characters.
const CF_ODD_VALUES: [u8; 36] = [
    1, 0, 5, 7, 9, 13, 15, 17, 19, 21, 1, 0, 5, 7, 9, 13, 15, 17, 19, 21, 2, 4, 18, 20, 11, 3, 6, 8, 12,
    14, 16, 10, 22, 25, 24, 23,
];
/// Codice fiscale check-character values for characters in even 1-based positions: digits keep
/// their value, letters count A=0 .. Z=25.
const CF_EVEN_VALUES: [u8; 36] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
    19, 20, 21, 22, 23, 24, 25,
];
/// Characters allowed in a codice fiscale digit slot (digits plus omocodia letters).
const CF_DIGIT_SLOT: &[u8] = b"0123456789LMNPQRSTUV";
/// Characters allowed for the codice fiscale month letter.
const CF_MONTH_SLOT: &[u8] = b"ABCDEHLMPRST";

/// Returns the boxes that cover sensitive data, in reading order.
pub fn find_sensitive(ocr: &OcrBoxes) -> Vec<Region> {
    let mut regions = Vec::new();
    for line in &ocr.lines {
        regions.extend(scan_line(line));
    }
    regions
}

fn scan_line(line: &OcrLine) -> Vec<Region> {
    let texts: Vec<&str> = line.words.iter().map(|word| clean_word(&word.text)).collect();
    let mut claimed = vec![false; texts.len()];
    let mut found: Vec<(usize, Region)> = Vec::new();
    for &(kind, max_words) in &DETECTORS {
        claim_windows(kind, max_words, &texts, &line.words, &mut claimed, &mut found);
    }
    found.sort_by_key(|(start, _)| *start);
    found.into_iter().map(|(_, region)| region).collect()
}

fn claim_windows(
    kind: SensitiveKind,
    max_words: usize,
    texts: &[&str],
    words: &[OcrWord],
    claimed: &mut [bool],
    found: &mut Vec<(usize, Region)>,
) {
    let mut start = 0;
    while start < texts.len() {
        let mut matched = None;
        if !claimed[start] {
            let limit = max_words.min(texts.len() - start);
            for len in (1..=limit).rev() {
                let end = start + len;
                if claimed[start..end].iter().any(|slot| *slot)
                    || texts[start..end].iter().any(|text| text.is_empty())
                {
                    continue;
                }
                if window_matches(kind, &texts[start..end]) {
                    matched = Some(len);
                    break;
                }
            }
        }
        match matched {
            Some(len) => {
                let end = start + len;
                for slot in &mut claimed[start..end] {
                    *slot = true;
                }
                found.push((start, union_region(kind, &words[start..end])));
                start = end;
            }
            None => start += 1,
        }
    }
}

fn window_matches(kind: SensitiveKind, window: &[&str]) -> bool {
    match kind {
        SensitiveKind::Iban => iban_window_valid(window),
        SensitiveKind::Card => card_window_valid(window),
        SensitiveKind::Email => email_window_valid(window),
        SensitiveKind::TaxCode => tax_code_window_valid(window),
        SensitiveKind::VatNumber => vat_window_valid(window),
        SensitiveKind::Phone => phone_window_valid(window),
    }
}

fn clean_word(raw: &str) -> &str {
    raw.trim()
        .trim_start_matches(LEADING_TRIM)
        .trim_end_matches(TRAILING_TRIM)
}

fn union_region(kind: SensitiveKind, words: &[OcrWord]) -> Region {
    let first = words.first().map_or([0.0; 4], |word| word.bbox);
    let mut x0 = first[0];
    let mut y0 = first[1];
    let mut x1 = first[0] + first[2];
    let mut y1 = first[1] + first[3];
    for word in words {
        let [x, y, w, h] = word.bbox;
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x + w);
        y1 = y1.max(y + h);
    }
    Region { kind, x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

fn iban_window_valid(window: &[&str]) -> bool {
    let mut joined = String::new();
    for text in window {
        joined.push_str(text);
    }
    iban_valid(strip_iban_label(&joined))
}

fn strip_iban_label(text: &str) -> &str {
    let mut rest = text;
    if rest
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("iban"))
    {
        rest = &rest[4..];
    }
    rest.trim_start_matches(':')
}

fn iban_valid(candidate: &str) -> bool {
    iban_shape(candidate) && iban_mod97(candidate) == 1
}

fn iban_shape(candidate: &str) -> bool {
    let bytes = candidate.as_bytes();
    (15..=34).contains(&bytes.len())
        && bytes[0].is_ascii_uppercase()
        && bytes[1].is_ascii_uppercase()
        && bytes[2].is_ascii_digit()
        && bytes[3].is_ascii_digit()
        && bytes[4..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || byte.is_ascii_uppercase())
}

fn iban_mod97(candidate: &str) -> u32 {
    let mut remainder: u32 = 0;
    let rearranged = candidate[4..].chars().chain(candidate[..4].chars());
    for ch in rearranged {
        let value = match ch {
            '0'..='9' => ch as u32 - '0' as u32,
            'A'..='Z' => ch as u32 - 'A' as u32 + 10,
            _ => return u32::MAX,
        };
        if value < 10 {
            remainder = (remainder * 10 + value) % 97;
        } else {
            remainder = (remainder * 100 + value) % 97;
        }
    }
    remainder
}

fn card_window_valid(window: &[&str]) -> bool {
    let mut digits = String::new();
    for text in window {
        if text.starts_with('-') || text.ends_with('-') {
            return false;
        }
        for ch in text.chars() {
            if ch == '-' {
                continue;
            }
            if !ch.is_ascii_digit() {
                return false;
            }
            digits.push(ch);
        }
    }
    card_digits_valid(&digits)
}

fn card_digits_valid(digits: &str) -> bool {
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let mut chars = digits.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if chars.all(|ch| ch == first) {
        return false;
    }
    luhn_valid(digits)
}

fn luhn_valid(digits: &str) -> bool {
    let mut sum: u32 = 0;
    let mut double = false;
    for ch in digits.chars().rev() {
        let mut value = ch.to_digit(10).unwrap_or(0);
        if double {
            value *= 2;
            if value > 9 {
                value -= 9;
            }
        }
        sum += value;
        double = !double;
    }
    sum.is_multiple_of(10)
}

fn email_window_valid(window: &[&str]) -> bool {
    match window {
        [word] => email_word_valid(word),
        _ => false,
    }
}

fn email_word_valid(word: &str) -> bool {
    let candidate = match word.find([':', '=']) {
        Some(index) => &word[index + 1..],
        None => word,
    };
    let Some((local, domain)) = candidate.split_once('@') else {
        return false;
    };
    if domain.contains('@') || !(1..=64).contains(&local.len()) {
        return false;
    }
    if !local
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "._%+-".contains(ch))
    {
        return false;
    }
    if !domain
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '.')
    {
        return false;
    }
    if !domain.split('.').all(|label| !label.is_empty()) {
        return false;
    }
    let Some((_, last)) = domain.rsplit_once('.') else {
        return false;
    };
    last.len() >= 2 && last.chars().all(|ch| ch.is_ascii_alphabetic())
}

fn tax_code_window_valid(window: &[&str]) -> bool {
    match window {
        [word] => tax_code_valid(word),
        _ => false,
    }
}

fn tax_code_valid(word: &str) -> bool {
    if word.len() != 16 || !word.is_ascii() {
        return false;
    }
    let upper = word.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let shape_ok = bytes[..6].iter().all(|byte| byte.is_ascii_uppercase())
        && CF_DIGIT_SLOT.contains(&bytes[6])
        && CF_DIGIT_SLOT.contains(&bytes[7])
        && CF_MONTH_SLOT.contains(&bytes[8])
        && CF_DIGIT_SLOT.contains(&bytes[9])
        && CF_DIGIT_SLOT.contains(&bytes[10])
        && bytes[11].is_ascii_uppercase()
        && CF_DIGIT_SLOT.contains(&bytes[12])
        && CF_DIGIT_SLOT.contains(&bytes[13])
        && CF_DIGIT_SLOT.contains(&bytes[14])
        && bytes[15].is_ascii_uppercase();
    shape_ok && cf_check_char(&upper[..15]) == Some(bytes[15])
}

fn cf_check_char(first15: &str) -> Option<u8> {
    let sum = first15
        .chars()
        .enumerate()
        .try_fold(0u32, |sum, (index, ch)| {
            cf_value(ch, index % 2 == 0).map(|value| sum + u32::from(value))
        })?;
    Some(b'A' + (sum % 26) as u8)
}

fn cf_value(ch: char, odd: bool) -> Option<u8> {
    let index = match ch {
        '0'..='9' => ch as usize - '0' as usize,
        'A'..='Z' => 10 + (ch as usize - 'A' as usize),
        _ => return None,
    };
    Some(if odd {
        CF_ODD_VALUES[index]
    } else {
        CF_EVEN_VALUES[index]
    })
}

fn vat_window_valid(window: &[&str]) -> bool {
    let mut joined = String::new();
    for text in window {
        joined.push_str(text);
    }
    vat_digits_valid(strip_it_prefix(&joined))
}

fn strip_it_prefix(text: &str) -> &str {
    if text
        .get(..2)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("it"))
    {
        &text[2..]
    } else {
        text
    }
}

fn vat_digits_valid(digits: &str) -> bool {
    if digits.len() != 11 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let values: Vec<u32> = digits.chars().map(|ch| ch as u32 - '0' as u32).collect();
    if values.iter().all(|&value| value == 0) {
        return false;
    }
    let sum: u32 = values[..10]
        .iter()
        .enumerate()
        .map(|(index, &digit)| {
            if index % 2 == 0 {
                digit
            } else {
                let doubled = digit * 2;
                if doubled > 9 {
                    doubled - 9
                } else {
                    doubled
                }
            }
        })
        .sum();
    (10 - sum % 10) % 10 == values[10]
}

fn phone_window_valid(window: &[&str]) -> bool {
    let mut digits = String::new();
    let mut leading_plus = false;
    for (word_index, text) in window.iter().enumerate() {
        for (char_index, ch) in text.chars().enumerate() {
            match ch {
                '+' if word_index == 0 && char_index == 0 => leading_plus = true,
                '+' => return false,
                '.' | '-' | '(' | ')' => {}
                _ if ch.is_ascii_digit() => digits.push(ch),
                _ => return false,
            }
        }
    }
    let length = digits.len();
    if leading_plus {
        return (8..=14).contains(&length);
    }
    if digits.starts_with("00") {
        return (10..=15).contains(&length);
    }
    match digits.as_bytes().first() {
        Some(b'3') => (9..=10).contains(&length),
        Some(b'0') => (7..=11).contains(&length),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IBAN: &str = "IT60X0542811101000000123456";
    const CARD: &str = "4111111111111111";
    const EMAIL: &str = "mario.rossi@example.com";
    const TAX_CODE: &str = "RSSMRA85T10A562S";
    const VAT: &str = "12345678903";

    fn word(text: &str, x: f64) -> OcrWord {
        OcrWord { text: text.to_string(), bbox: [x, 0.20, 0.08, 0.04] }
    }

    fn line_of(texts: &[&str]) -> OcrLine {
        OcrLine {
            words: texts
                .iter()
                .enumerate()
                .map(|(index, text)| word(text, 0.05 + index as f64 * 0.09))
                .collect(),
        }
    }

    fn one_line(texts: &[&str]) -> OcrBoxes {
        OcrBoxes { width: 1000, height: 1000, lines: vec![line_of(texts)] }
    }

    fn regions(texts: &[&str]) -> Vec<Region> {
        find_sensitive(&one_line(texts))
    }

    fn kinds(texts: &[&str]) -> Vec<SensitiveKind> {
        regions(texts).iter().map(|region| region.kind).collect()
    }

    #[test]
    fn deserializes_bridge_json() {
        let json = r#"{"ok":true,"width":800,"height":600,"lines":[{"words":[{"text":"IT60","box":[0.1,0.2,0.3,0.04]}]}]}"#;
        let ocr: OcrBoxes = serde_json::from_str(json).expect("valid bridge JSON");
        assert_eq!(ocr.width, 800);
        assert_eq!(ocr.height, 600);
        assert_eq!(ocr.lines[0].words[0].bbox, [0.1, 0.2, 0.3, 0.04]);
    }

    #[test]
    fn empty_document_has_no_regions() {
        let ocr = OcrBoxes { width: 0, height: 0, lines: Vec::new() };
        assert!(find_sensitive(&ocr).is_empty());
    }

    #[test]
    fn iban_checks_the_value() {
        assert!(iban_valid(IBAN));
        assert!(!iban_valid("IT60X0542811101000000123457"));
        assert!(!iban_valid("IT60 X054 2811 1010 0000 0123 456"));
        assert!(!iban_valid("IBAN"));
    }

    #[test]
    fn iban_one_word() {
        assert_eq!(kinds(&[IBAN]), vec![SensitiveKind::Iban]);
    }

    #[test]
    fn iban_split_across_words() {
        let found = regions(&["IT60", "X054", "2811", "1010", "0000", "0123", "456"]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, SensitiveKind::Iban);
    }

    #[test]
    fn iban_label_glued_and_separate() {
        let glued = format!("IBAN:{IBAN}");
        assert_eq!(kinds(&[&glued]), vec![SensitiveKind::Iban]);
        assert_eq!(
            kinds(&["IBAN", "IT60", "X054", "2811", "1010", "0000", "0123", "456"]),
            vec![SensitiveKind::Iban]
        );
    }

    #[test]
    fn iban_label_alone_is_not_covered() {
        assert!(regions(&["IBAN"]).is_empty());
        assert!(regions(&["IBAN:", "EUR"]).is_empty());
    }

    #[test]
    fn iban_two_values_on_one_line() {
        assert_eq!(kinds(&[IBAN, IBAN]), vec![SensitiveKind::Iban, SensitiveKind::Iban]);
    }

    #[test]
    fn iban_bad_check_is_rejected() {
        assert!(regions(&["IT60X0542811101000000123457"]).is_empty());
    }

    #[test]
    fn iban_union_box_covers_all_words() {
        let ocr = OcrBoxes {
            width: 100,
            height: 100,
            lines: vec![OcrLine {
                words: vec![
                    OcrWord { text: "IT60".into(), bbox: [0.10, 0.30, 0.20, 0.04] },
                    OcrWord { text: "X054".into(), bbox: [0.32, 0.28, 0.20, 0.07] },
                    OcrWord { text: "2811101000000123456".into(), bbox: [0.55, 0.31, 0.30, 0.03] },
                ],
            }],
        };
        let found = find_sensitive(&ocr);
        assert_eq!(found.len(), 1);
        let region = found[0];
        assert!((region.x - 0.10).abs() < 1e-9);
        assert!((region.y - 0.28).abs() < 1e-9);
        assert!((region.w - 0.75).abs() < 1e-9);
        assert!((region.h - 0.07).abs() < 1e-9);
    }

    #[test]
    fn luhn_checks_the_value() {
        assert!(luhn_valid(CARD));
        assert!(!luhn_valid("4111111111111112"));
    }

    #[test]
    fn card_one_word() {
        assert_eq!(kinds(&[CARD]), vec![SensitiveKind::Card]);
    }

    #[test]
    fn card_split_across_words() {
        let found = regions(&["4111", "1111", "1111", "1111"]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, SensitiveKind::Card);
    }

    #[test]
    fn card_with_hyphens_and_label_prefix() {
        assert_eq!(kinds(&["4111-1111-1111-1111"]), vec![SensitiveKind::Card]);
        assert_eq!(kinds(&["Card:", CARD]), vec![SensitiveKind::Card]);
    }

    #[test]
    fn card_two_values_on_one_line() {
        assert_eq!(kinds(&[CARD, CARD]), vec![SensitiveKind::Card, SensitiveKind::Card]);
    }

    #[test]
    fn card_bad_luhn_is_rejected() {
        assert!(regions(&["4111111111111112"]).is_empty());
    }

    #[test]
    fn card_with_all_same_digits_is_rejected() {
        assert!(!card_digits_valid("1111111111111111"));
        assert!(!card_digits_valid("411111111111111"));
    }

    #[test]
    fn card_is_never_reported_as_phone() {
        assert_eq!(kinds(&[CARD]), vec![SensitiveKind::Card]);
    }

    #[test]
    fn email_checks_the_value() {
        assert!(email_word_valid(EMAIL));
        assert!(email_word_valid("email:mario.rossi@example.com"));
        assert!(!email_word_valid("mario.rossi@example"));
        assert!(!email_word_valid("@example.com"));
        assert!(!email_word_valid("mario@example.c"));
    }

    #[test]
    fn email_one_word() {
        assert_eq!(kinds(&[EMAIL]), vec![SensitiveKind::Email]);
    }

    #[test]
    fn email_with_label_prefix() {
        assert_eq!(kinds(&["email:mario.rossi@example.com"]), vec![SensitiveKind::Email]);
        assert_eq!(kinds(&["mail=", EMAIL]), vec![SensitiveKind::Email]);
    }

    #[test]
    fn email_two_values_on_one_line() {
        assert_eq!(kinds(&[EMAIL, EMAIL]), vec![SensitiveKind::Email, SensitiveKind::Email]);
    }

    #[test]
    fn email_negatives_are_rejected() {
        assert!(regions(&["mario.rossi@example"]).is_empty());
        assert!(regions(&["mario@example.c"]).is_empty());
    }

    #[test]
    fn email_split_across_words_is_not_matched() {
        assert!(regions(&["mario.rossi@", "example.com"]).is_empty());
    }

    #[test]
    fn tax_code_checks_the_value() {
        assert!(tax_code_valid(TAX_CODE));
        assert!(tax_code_valid("rssmra85t10a562s"));
        assert!(!tax_code_valid("RSSMRA85T10A562X"));
        assert!(!tax_code_valid("RSSMRA85T10A562"));
    }

    #[test]
    fn tax_code_one_word() {
        assert_eq!(kinds(&[TAX_CODE]), vec![SensitiveKind::TaxCode]);
    }

    #[test]
    fn tax_code_with_label_prefix() {
        assert_eq!(kinds(&["CF:", TAX_CODE]), vec![SensitiveKind::TaxCode]);
    }

    #[test]
    fn tax_code_two_values_on_one_line() {
        assert_eq!(kinds(&[TAX_CODE, TAX_CODE]), vec![SensitiveKind::TaxCode, SensitiveKind::TaxCode]);
    }

    #[test]
    fn tax_code_bad_check_is_rejected() {
        assert!(regions(&["RSSMRA85T10A562X"]).is_empty());
    }

    #[test]
    fn tax_code_split_across_words_is_not_matched() {
        assert!(regions(&["RSSMRA85T1", "0A562S"]).is_empty());
    }

    #[test]
    fn vat_checks_the_value() {
        assert!(vat_digits_valid(VAT));
        assert!(!vat_digits_valid("12345678904"));
        assert!(!vat_digits_valid("00000000000"));
    }

    #[test]
    fn vat_one_word() {
        assert_eq!(kinds(&[VAT]), vec![SensitiveKind::VatNumber]);
    }

    #[test]
    fn vat_split_across_words() {
        let found = regions(&["123", "456", "78903"]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, SensitiveKind::VatNumber);
    }

    #[test]
    fn vat_with_it_prefix_and_label() {
        assert_eq!(kinds(&["IT12345678903"]), vec![SensitiveKind::VatNumber]);
        assert_eq!(kinds(&["IT", VAT]), vec![SensitiveKind::VatNumber]);
        assert_eq!(kinds(&["P.IVA:", VAT]), vec![SensitiveKind::VatNumber]);
    }

    #[test]
    fn vat_two_values_on_one_line() {
        assert_eq!(kinds(&[VAT, VAT]), vec![SensitiveKind::VatNumber, SensitiveKind::VatNumber]);
    }

    #[test]
    fn vat_bad_check_is_rejected() {
        assert!(regions(&["12345678904"]).is_empty());
    }

    #[test]
    fn phone_checks_the_value() {
        assert!(phone_window_valid(&["+39", "333", "1234567"]));
        assert!(phone_window_valid(&["+393331234567"]));
        assert!(phone_window_valid(&["06", "1234", "5678"]));
        assert!(!phone_window_valid(&["12/03/2025"]));
        assert!(!phone_window_valid(&["1.234,56"]));
    }

    #[test]
    fn phone_known_examples() {
        let found = regions(&["+39", "333", "1234567"]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, SensitiveKind::Phone);
        assert_eq!(kinds(&["333", "1234567"]), vec![SensitiveKind::Phone]);
        assert_eq!(kinds(&["06", "1234", "5678"]), vec![SensitiveKind::Phone]);
    }

    #[test]
    fn phone_with_label_prefix() {
        assert_eq!(kinds(&["Tel.", "+39", "333", "1234567"]), vec![SensitiveKind::Phone]);
    }

    #[test]
    fn phone_two_values_on_one_line() {
        assert_eq!(
            kinds(&["+39", "333", "1234567", "+39", "333", "1234567"]),
            vec![SensitiveKind::Phone, SensitiveKind::Phone]
        );
    }

    #[test]
    fn dates_amounts_years_and_currency_are_not_covered() {
        for text in ["12/03/2025", "1.234,56", "2025", "EUR"] {
            assert!(regions(&[text]).is_empty(), "{text} must stay uncovered");
        }
    }

    #[test]
    fn nine_digit_number_starting_with_one_is_not_covered() {
        assert!(regions(&["123456789"]).is_empty());
    }

    #[test]
    fn regions_come_out_in_reading_order() {
        // The email detector runs before the phone one, but the phone is further left.
        assert_eq!(
            kinds(&["333", "1234567", EMAIL]),
            vec![SensitiveKind::Phone, SensitiveKind::Email]
        );
        let two_lines = OcrBoxes {
            width: 1000,
            height: 1000,
            lines: vec![line_of(&[EMAIL]), line_of(&[VAT])],
        };
        let found = find_sensitive(&two_lines);
        assert_eq!(
            found.iter().map(|region| region.kind).collect::<Vec<_>>(),
            vec![SensitiveKind::Email, SensitiveKind::VatNumber]
        );
    }
}
