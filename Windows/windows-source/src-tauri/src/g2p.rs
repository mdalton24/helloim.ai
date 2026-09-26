//! Turning written English into the phonemes Kokoro actually speaks.
//!
//! Kokoro's ONNX graph does not take text. It takes token ids over a 114-symbol
//! IPA alphabet, and something has to get from "the weather is fine" to
//! `ðə wˈɛðɚ ɪz fˈaɪn`. That step is the whole reason a text-to-speech model is
//! not just a file you load.
//!
//! WHY THIS IS A DICTIONARY AND NOT espeak-ng. The reference pipeline reaches
//! for espeak-ng, which is **GPLv3** — linking it into a product Mark intends to
//! sell would put the whole app under the GPL. That is a business decision, not
//! a technical one, and it is not mine to make quietly by choosing a dependency.
//! misaki's pronunciation dictionaries are **Apache 2.0**, cover 178,399 words,
//! and need no runtime at all.
//!
//! WHAT IT GIVES UP, said plainly rather than discovered later:
//!   * **Homographs.** "read", "lead", "live" have two pronunciations chosen by
//!     part of speech. misaki uses spaCy for that; there is no spaCy here, so
//!     the default reading wins. Occasionally it will say "reed" for "red".
//!   * **Unknown words.** A word in neither dictionary falls through to a
//!     letter-to-sound guess. It will be wrong sometimes, and wrong out loud is
//!     better than silent — the alternative is dropping the word entirely and
//!     leaving a hole in the sentence with no clue why.

use std::collections::HashMap;
use std::sync::OnceLock;

/// word -> IPA, built once. 178k entries at about 4.4 MB of text; parsing it
/// takes a few tens of milliseconds and happens on the first spoken line, not
/// at startup, so it never delays the window opening.
static DICT: OnceLock<HashMap<String, String>> = OnceLock::new();
static VOCAB: OnceLock<HashMap<char, i64>> = OnceLock::new();

const DICT_TSV: &str = include_str!("../assets/us_phonemes.tsv");
const VOCAB_JSON: &str = include_str!("../assets/vocab.json");

fn dict() -> &'static HashMap<String, String> {
    DICT.get_or_init(|| {
        let mut m = HashMap::with_capacity(180_000);
        for line in DICT_TSV.lines() {
            if let Some((w, p)) = line.split_once('\t') {
                m.insert(w.to_string(), p.to_string());
            }
        }
        m
    })
}

fn vocab() -> &'static HashMap<char, i64> {
    VOCAB.get_or_init(|| {
        let raw: HashMap<String, i64> = serde_json::from_str(VOCAB_JSON).unwrap_or_default();
        raw.into_iter()
            .filter_map(|(k, v)| k.chars().next().map(|c| (c, v)))
            .collect()
    })
}

/// Numbers, read the way a person reads them.
///
/// "1999" is "nineteen ninety-nine" in a year and "one thousand nine hundred
/// and ninety-nine" in a total, and nothing here can tell which. Digit-by-digit
/// would be worse than both, so this expands cardinals and accepts that years
/// come out long.
fn say_number(n: i64) -> String {
    const ONES: [&str; 20] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight",
        "nine", "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen",
        "sixteen", "seventeen", "eighteen", "nineteen",
    ];
    const TENS: [&str; 10] = [
        "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy",
        "eighty", "ninety",
    ];
    if n < 0 {
        return format!("minus {}", say_number(-n));
    }
    match n {
        0..=19 => ONES[n as usize].to_string(),
        20..=99 => {
            let (t, o) = ((n / 10) as usize, n % 10);
            if o == 0 { TENS[t].to_string() } else { format!("{} {}", TENS[t], ONES[o as usize]) }
        }
        100..=999 => {
            let (h, r) = (n / 100, n % 100);
            if r == 0 { format!("{} hundred", say_number(h)) }
            else { format!("{} hundred and {}", say_number(h), say_number(r)) }
        }
        1_000..=999_999 => {
            let (t, r) = (n / 1_000, n % 1_000);
            if r == 0 { format!("{} thousand", say_number(t)) }
            else { format!("{} thousand {}", say_number(t), say_number(r)) }
        }
        _ => {
            let (m, r) = (n / 1_000_000, n % 1_000_000);
            if r == 0 { format!("{} million", say_number(m)) }
            else { format!("{} million {}", say_number(m), say_number(r)) }
        }
    }
}

/// "26th", not "twenty six th".
fn say_ordinal(n: i64) -> String {
    let words = say_number(n);
    let (head, last) = match words.rsplit_once(' ') {
        Some((h, l)) => (Some(h), l),
        None => (None, words.as_str()),
    };
    let ord = match last {
        "one" => "first".into(),
        "two" => "second".into(),
        "three" => "third".into(),
        "five" => "fifth".into(),
        "eight" => "eighth".into(),
        "nine" => "ninth".into(),
        "twelve" => "twelfth".into(),
        // twenty → twentieth, forty → fortieth. The tens are the only words
        // that end in y here, so this needs no list of its own.
        w if w.ends_with('y') => format!("{}ieth", &w[..w.len() - 1]),
        w => format!("{w}th"),
    };
    match head {
        Some(h) => format!("{h} {ord}"),
        None => ord,
    }
}

/// Read a written number starting at `at`: digits, thousands commas, and a
/// decimal part if a dot is genuinely followed by a digit.
///
/// The dot test is the whole reason this is a function. "The total was 1,250."
/// ends a sentence; "1,250.50" does not, and the only difference is what comes
/// after the dot.
fn read_amount(c: &[char], at: usize) -> (Option<i64>, Option<String>, usize) {
    let mut digits = String::new();
    let mut i = at;
    while i < c.len() {
        if c[i].is_ascii_digit() {
            digits.push(c[i]);
            i += 1;
        } else if c[i] == ',' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
            i += 1;
        } else {
            break;
        }
    }
    let mut frac = None;
    if c.get(i) == Some(&'.') && c.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
        let mut f = String::new();
        i += 1;
        while i < c.len() && c[i].is_ascii_digit() {
            f.push(c[i]);
            i += 1;
        }
        frac = Some(f);
    }
    (digits.parse::<i64>().ok(), frac, i)
}

/// Money, ordinals, decimals and percentages, before a single word is looked up.
///
/// THIS HAS TO SEE THE WHOLE STRING, which is why it is not part of the word
/// loop below. Everything that breaks here spans a punctuation mark: "$1,250" is
/// a symbol, two digit runs and a comma, and the loop sees four separate pieces.
/// It said "one, two hundred and fifty" until this existed — found by
/// transcribing the spoken audio back with whisper, not by reading the code,
/// which is the only way a fault like this ever surfaces.
fn normalize(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        // The sign is written first and spoken last.
        if c[i] == '$' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
            if let (Some(whole), frac, next) = read_amount(&c, i + 1) {
                out.push_str(&say_number(whole));
                out.push_str(if whole == 1 { " dollar" } else { " dollars" });
                // ".5" is fifty cents, not five. Two places, padded not parsed.
                let cents: i64 = frac
                    .map(|f| format!("{f:0<2}")[..2].parse().unwrap_or(0))
                    .unwrap_or(0);
                if cents > 0 {
                    out.push_str(&format!(
                        " and {} {}",
                        say_number(cents),
                        if cents == 1 { "cent" } else { "cents" }
                    ));
                }
                i = next;
                continue;
            }
        }
        if c[i].is_ascii_digit() {
            if let (Some(whole), frac, mut next) = read_amount(&c, i) {
                let suffix: String =
                    c[next..(next + 2).min(c.len())].iter().collect::<String>().to_lowercase();
                // "26th" is one thing. "3D" is not — so the suffix has to be
                // exactly the ordinal ending, with no word carrying on after it.
                let is_ordinal = frac.is_none()
                    && matches!(suffix.as_str(), "st" | "nd" | "rd" | "th")
                    && !c.get(next + 2).is_some_and(|ch| ch.is_alphanumeric());
                if is_ordinal {
                    out.push_str(&say_ordinal(whole));
                    next += 2;
                } else {
                    out.push_str(&say_number(whole));
                    if let Some(f) = frac {
                        // Digit by digit after the point: 3.05 is "three point
                        // zero five", never "three point five".
                        out.push_str(" point");
                        for d in f.chars() {
                            out.push(' ');
                            out.push_str(&say_number(d.to_digit(10).unwrap_or(0) as i64));
                        }
                    }
                }
                if c.get(next) == Some(&'%') {
                    out.push_str(" percent");
                    next += 1;
                }
                i = next;
                continue;
            }
        }
        out.push(c[i]);
        i += 1;
    }
    out
}

/// A guess for a word in neither dictionary.
///
/// DELIBERATELY CRUDE, and deliberately not silent. Proper letter-to-sound
/// needs a model; this is a handful of English spelling regularities. It will
/// mispronounce unusual names. The alternative — dropping the word — leaves a
/// hole in a sentence with nothing to explain it, and a listener cannot tell a
/// dropped word from a model that broke.
fn guess(word: &str) -> String {
    let chars: Vec<char> = word.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        let pair = match two.as_str() {
            "ch" => Some("ʧ"), "sh" => Some("ʃ"), "th" => Some("θ"),
            "ph" => Some("f"), "ck" => Some("k"), "ng" => Some("ŋ"),
            "oo" => Some("uː"), "ee" => Some("iː"), "ea" => Some("iː"),
            "ou" => Some("aʊ"), "ow" => Some("aʊ"), "ai" => Some("eɪ"),
            "ay" => Some("eɪ"), "oa" => Some("oʊ"), "qu" => Some("kw"),
            _ => None,
        };
        if let Some(p) = pair {
            out.push_str(p);
            i += 2;
            continue;
        }
        let c = chars[i];
        out.push_str(match c {
            'a' => "æ", 'e' => "ɛ", 'i' => "ɪ", 'o' => "ɑ", 'u' => "ʌ",
            'y' => "i", 'c' => "k", 'q' => "k", 'x' => "ks", 'j' => "ʤ",
            'g' => "ɡ", 'r' => "ɹ",
            _ => {
                out.push(c);
                i += 1;
                continue;
            }
        });
        i += 1;
    }
    out
}

/// Text in, phoneme string out.
pub fn phonemize(text: &str) -> String {
    let text = &normalize(text);
    let d = dict();
    let mut out = String::new();
    // Punctuation is IN the vocabulary and is what gives a sentence its shape —
    // dropping commas and full stops is a large part of why synthetic speech
    // sounds like a list rather than a person, so they are passed straight
    // through rather than stripped with everything else.
    let mut word = String::new();

    let flush = |word: &mut String, out: &mut String, rest: &str| {
        if word.is_empty() {
            return;
        }
        let lower = word.to_lowercase();
        let ph = if let Ok(n) = lower.replace(',', "").parse::<i64>() {
            // Numbers become words, then each of those is looked up.
            say_number(n)
                .split(' ')
                .map(|w| d.get(w).cloned().unwrap_or_else(|| guess(w)))
                .collect::<Vec<_>>()
                .join(" ")
        } else if lower == "the" {
            /* "the" IS THE COMMONEST WORD IN ENGLISH and it has two readings:
               ðə before a consonant, ði before a vowel. The dictionary holds one
               of them, so every sentence inherited whichever it happened to be —
               checked against the reference pipeline, which gets this right.
               Decided by what comes NEXT, which is why it is handled here and
               not by a lookup. */
            let next_is_vowel = rest
                .trim_start()
                .chars()
                .next()
                .is_some_and(|c| matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u'));
            if next_is_vowel { "ði".to_string() } else { "ðə".to_string() }
        } else if let Some(p) = d.get(&lower) {
            p.clone()
        } else if let Some(stem) = lower.strip_suffix("'s").or_else(|| lower.strip_suffix('s')) {
            // A plural or possessive whose stem is known is far commoner than a
            // genuinely unknown word, and worth one extra lookup before guessing.
            match d.get(stem) {
                Some(p) => format!("{p}z"),
                None => guess(&lower),
            }
        } else {
            guess(&lower)
        };
        if !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
        out.push_str(&ph);
        word.clear();
    };

    for (idx, c) in text.char_indices() {
        if c.is_alphanumeric() || c == '\'' {
            word.push(c);
        } else {
            flush(&mut word, &mut out, &text[idx + c.len_utf8()..]);
            if matches!(c, ',' | '.' | '!' | '?' | ';' | ':' | '—' | '…' | '"' | '(' | ')') {
                out.push(c);
            } else if c.is_whitespace() && !out.ends_with(' ') {
                out.push(' ');
            }
        }
    }
    flush(&mut word, &mut out, "");
    out.trim().to_string()
}

/// Phonemes to the model's token ids.
///
/// The 0 either side is not padding for tidiness — the model was trained with
/// it and produces noticeably worse audio at the edges without it.
pub fn tokens(phonemes: &str) -> Vec<i64> {
    let v = vocab();
    let mut ids: Vec<i64> = vec![0];
    ids.extend(phonemes.chars().filter_map(|c| v.get(&c).copied()));
    ids.push(0);
    ids
}

/// The model's positional encoding is fixed at 510 tokens plus the two pads, so
/// anything longer has to be spoken in pieces. Split on sentence ends, and only
/// mid-sentence when a single sentence is somehow longer than the window.
pub fn split_for_model(phonemes: &str, max: usize) -> Vec<String> {
    if phonemes.chars().count() <= max {
        return vec![phonemes.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    for part in phonemes.split_inclusive(['.', '!', '?', ';']) {
        if cur.chars().count() + part.chars().count() > max && !cur.is_empty() {
            out.push(cur.trim().to_string());
            cur = String::new();
        }
        if part.chars().count() > max {
            // One enormous sentence. Break it on spaces rather than mid-phoneme,
            // which would invent sounds that were never in the text.
            for w in part.split_inclusive(' ') {
                if cur.chars().count() + w.chars().count() > max && !cur.is_empty() {
                    out.push(cur.trim().to_string());
                    cur = String::new();
                }
                cur.push_str(w);
            }
        } else {
            cur.push_str(part);
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_takes_its_vowel_form_before_a_vowel() {
        // Checked against the reference pipeline, which does this correctly.
        assert!(phonemize("the weather").starts_with('ð'));
        assert!(phonemize("the weather").starts_with("ðə"), "{}", phonemize("the weather"));
        assert!(phonemize("the apple").starts_with("ði"), "{}", phonemize("the apple"));
    }

    #[test]
    fn known_words_come_from_the_dictionary() {
        let p = phonemize("hello world");
        assert!(p.contains('ə') || p.contains('ˈ'), "got {p}");
        assert!(!p.is_empty());
    }

    #[test]
    fn punctuation_survives_because_it_is_the_phrasing() {
        let p = phonemize("wait, really?");
        assert!(p.contains(','), "the comma is what makes it a pause: {p}");
        assert!(p.contains('?'));
    }

    #[test]
    fn numbers_are_spoken_not_spelled() {
        assert_eq!(say_number(0), "zero");
        assert_eq!(say_number(19), "nineteen");
        assert_eq!(say_number(42), "forty two");
        assert_eq!(say_number(100), "one hundred");
        assert_eq!(say_number(1999), "one thousand nine hundred and ninety nine");
        assert_eq!(say_number(-5), "minus five");
        // And they reach the phonemizer as words, not as digits.
        let p = phonemize("42");
        assert!(!p.contains('4') && !p.contains('2'), "digits leaked: {p}");
    }

    #[test]
    fn an_unknown_word_is_guessed_rather_than_dropped() {
        let p = phonemize("zzblorptastic");
        assert!(!p.is_empty(), "a hole in the sentence is worse than a bad guess");
    }

    #[test]
    fn every_phoneme_maps_to_a_token() {
        let ids = tokens(&phonemize("the quick brown fox jumps over the lazy dog."));
        assert!(ids.len() > 10);
        assert_eq!(ids.first(), Some(&0));
        assert_eq!(ids.last(), Some(&0));
    }

    #[test]
    fn long_text_is_split_under_the_window() {
        let long = phonemize(&"this is a sentence. ".repeat(80));
        let parts = split_for_model(&long, 510);
        assert!(parts.len() > 1);
        for p in &parts {
            assert!(p.chars().count() <= 510, "a chunk is over the model's window");
        }
    }

    /* THESE FOUR ARE A SCAR, 2026-08-26. The model spoke fluently and said the
       wrong thing, and the code looked right the whole time. It only surfaced
       because the audio was transcribed back: "$1,250" came out as "one, two
       hundred and fifty", and "26th" leaked the raw digits into the phonemes. */

    #[test]
    fn money_is_read_as_money() {
        assert_eq!(normalize("$1,250"), "one thousand two hundred and fifty dollars");
        assert_eq!(
            normalize("$1,250.50"),
            "one thousand two hundred and fifty dollars and fifty cents"
        );
        // ".5" is fifty cents. Parsing the fraction as a number gives five.
        assert_eq!(normalize("$3.5"), "three dollars and fifty cents");
        assert_eq!(normalize("$1"), "one dollar");
        assert_eq!(normalize("$0.01"), "zero dollars and one cent");
        // And no digit reaches the model.
        let p = phonemize("it cost $1,250");
        assert!(!p.chars().any(|c| c.is_ascii_digit()), "digits leaked: {p}");
    }

    #[test]
    fn ordinals_are_ordinals() {
        assert_eq!(normalize("1st"), "first");
        assert_eq!(normalize("2nd"), "second");
        assert_eq!(normalize("3rd"), "third");
        assert_eq!(normalize("26th"), "twenty sixth");
        assert_eq!(normalize("40th"), "fortieth");
        assert_eq!(normalize("112th"), "one hundred and twelfth");
        // "3D" is not an ordinal, and neither is a number with a word stuck on.
        assert_eq!(normalize("3D"), "threeD");
        assert_eq!(normalize("2nds"), "twonds");
    }

    #[test]
    fn a_full_stop_is_not_a_decimal_point() {
        // The only thing telling these apart is whether a digit follows the dot.
        assert_eq!(normalize("The total was 1,250."), "The total was one thousand two hundred and fifty.");
        assert_eq!(normalize("3.05"), "three point zero five");
        assert_eq!(normalize("12%"), "twelve percent");
    }

    #[test]
    fn a_thousands_comma_does_not_split_the_number() {
        // The word loop below breaks on a comma, so this has to be gone first.
        assert_eq!(normalize("1,250"), "one thousand two hundred and fifty");
        // A comma that is punctuation still is.
        assert_eq!(normalize("wait, 5 things"), "wait, five things");
    }

    #[test]
    fn plurals_fall_back_to_their_stem() {
        // "widgets" is unlikely to be in the dictionary; "widget" is.
        let p = phonemize("widgets");
        assert!(!p.is_empty());
    }
}
