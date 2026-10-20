//! Time-based one-time passwords. An entry keeps one in either of the ways
//! KeePass-format apps do: an "otp" field with an `otpauth://totp/…` link,
//! or KeePass's own "TimeOtp-*" fields. Both are read into one link with
//! every setting spelled out (6 digits, 30 seconds and SHA-1 unless it says
//! otherwise), and the keepass crate decodes its secret and computes the
//! code. A change is always written as an "otp" link.
//!
//! The link holds the secret: it is a protected field, kept in `Zeroizing`
//! here, and never logged or shown. (Parsing it makes copies inside the URL
//! crate that aren't wiped; the keepass crate's own parser does the same.)

use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use keepass::db::{Entry, TOTP};
use url::Url;
use zeroize::Zeroizing;

use crate::vault::fields;

/// KeePass's own fields: the secret in one of four encodings, and the settings.
pub const TIME_OTP_FIELDS: [&str; 7] = [
    "TimeOtp-Secret",
    "TimeOtp-Secret-Hex",
    "TimeOtp-Secret-Base32",
    "TimeOtp-Secret-Base64",
    "TimeOtp-Length",
    "TimeOtp-Period",
    "TimeOtp-Algorithm",
];

/// Why a one-time password can't be used. The messages never quote it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalid {
    /// A counter-based (HOTP) or other kind of one-time password.
    NotTimeBased,
    /// A setup this app can't compute codes for (another encoding, such as a game platform's).
    Unsupported,
    /// The secret is missing or not in base32.
    Secret,
    /// The digits, period or algorithm aren't valid.
    Settings,
}

impl Invalid {
    pub fn message(self) -> &'static str {
        match self {
            Invalid::NotTimeBased => "Only time-based one-time passwords are supported. Use the setup link for an authenticator app",
            Invalid::Unsupported => "This kind of one-time password isn't supported. Use a standard setup link",
            Invalid::Secret => "The one-time password secret isn't valid. Paste the otpauth:// link or the secret the site showed",
            Invalid::Settings => "The one-time password settings aren't valid. Paste the setup link again",
        }
    }
}

/// A code and how long it stays valid.
pub struct Code {
    pub code: Zeroizing<String>,
    /// Whole seconds left (1 to `period`).
    pub left: u64,
    pub period: u64,
}

impl Code {
    /// "123 456", "1234 5678": easier to read off and type.
    pub fn grouped(&self) -> Zeroizing<String> {
        group(&self.code)
    }
}

/// The digits in groups ("123 456"); `•` for each when hidden.
pub fn group(code: &str) -> Zeroizing<String> {
    let n = code.chars().count();
    let size = match n {
        6 | 9 => 3,
        8 => 4,
        _ => n.max(1),
    };
    let mut out = Zeroizing::new(String::with_capacity(n + n / size));
    for (i, ch) in code.chars().enumerate() {
        if i > 0 && i % size == 0 {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}

/// The entry's one-time password as stored: the "otp" field, else a link
/// made from the TimeOtp fields. `None`: it has none.
pub fn stored(e: &Entry) -> Option<Result<Zeroizing<String>, Invalid>> {
    if let Some(v) = e.get(fields::OTP).filter(|v| !v.trim().is_empty()) {
        return Some(Ok(Zeroizing::new(v.to_string())));
    }
    if TIME_OTP_FIELDS[..4].iter().all(|f| e.get(f).is_none_or(str::is_empty)) {
        return None;
    }
    Some(from_time_otp(e))
}

/// The entry's current code. `None`: it has no one-time password.
pub fn code_now(e: &Entry) -> Option<Result<Code, Invalid>> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    stored(e).map(|s| s.and_then(|link| code_at(&link, now)))
}

/// The code for a stored link or secret at `unix` seconds.
pub fn code_at(stored: &str, unix: u64) -> Result<Code, Invalid> {
    let link = normalize(stored, "")?;
    let totp = TOTP::from_str(&link).map_err(|_| Invalid::Settings)?;
    let v = totp.value_at(unix);
    Ok(Code { code: Zeroizing::new(v.code.clone()), left: v.valid_for.as_secs(), period: v.period.as_secs() })
}

/// What was typed or pasted (an otpauth link, or the bare secret as sites
/// show it, in any case and with spaces) as a link with every setting
/// spelled out; `label` names it when there is no link to take it from.
pub fn normalize(text: &str, label: &str) -> Result<Zeroizing<String>, Invalid> {
    let text = text.trim();
    let is_link = text.get(..10).is_some_and(|s| s.eq_ignore_ascii_case("otpauth://"));
    if !is_link {
        let secret = base32_secret(text)?;
        return Ok(link(label, &secret, 6, 30, "SHA1", None));
    }
    let url = Url::parse(text).map_err(|_| Invalid::Settings)?;
    if !url.host_str().is_some_and(|h| h.eq_ignore_ascii_case("totp")) {
        return Err(Invalid::NotTimeBased);
    }
    let (mut secret, mut digits, mut period, mut algorithm, mut issuer) = (None, 6, 30, "SHA1".to_string(), None);
    for (k, v) in url.query_pairs() {
        match k.to_ascii_lowercase().as_str() {
            "secret" => secret = Some(base32_secret(&v)?),
            "digits" => digits = v.trim().parse().map_err(|_| Invalid::Settings)?,
            "period" => period = v.trim().parse().map_err(|_| Invalid::Settings)?,
            "algorithm" => algorithm = v.trim().to_ascii_uppercase(),
            "issuer" => issuer = Some(v.into_owned()),
            "encoder" if !v.is_empty() => return Err(Invalid::Unsupported),
            _ => {}
        }
    }
    let secret = secret.ok_or(Invalid::Secret)?;
    let label = percent_decoded(url.path().trim_start_matches('/'));
    check(digits, period, &algorithm)?;
    Ok(link(&label, &secret, digits, period, &algorithm, issuer.as_deref()))
}

/// KeePass's fields as a link.
fn from_time_otp(e: &Entry) -> Result<Zeroizing<String>, Invalid> {
    let get = |f: &str| e.get(f).map(str::trim).filter(|v| !v.is_empty());
    let bytes: Zeroizing<Vec<u8>> = if let Some(s) = get("TimeOtp-Secret-Base32") {
        let s = base32_secret(s)?;
        return time_otp_link(e, &s);
    } else if let Some(s) = get("TimeOtp-Secret") {
        Zeroizing::new(s.as_bytes().to_vec())
    } else if let Some(s) = get("TimeOtp-Secret-Hex") {
        let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        Zeroizing::new(hex::decode(Zeroizing::new(s).as_str()).map_err(|_| Invalid::Secret)?)
    } else if let Some(s) = get("TimeOtp-Secret-Base64") {
        Zeroizing::new(base64::engine::general_purpose::STANDARD.decode(s).map_err(|_| Invalid::Secret)?)
    } else {
        return Err(Invalid::Secret);
    };
    if bytes.is_empty() {
        return Err(Invalid::Secret);
    }
    let s = Zeroizing::new(base32::encode(base32::Alphabet::Rfc4648 { padding: true }, &bytes));
    time_otp_link(e, &s)
}

fn time_otp_link(e: &Entry, secret: &str) -> Result<Zeroizing<String>, Invalid> {
    let num = |f: &str, default: u64| e.get(f).map_or(Ok(default), |v| v.trim().parse::<u64>().map_err(|_| Invalid::Settings));
    let digits = u32::try_from(num("TimeOtp-Length", 6)?).map_err(|_| Invalid::Settings)?;
    let period = num("TimeOtp-Period", 30)?;
    let algorithm = match e.get("TimeOtp-Algorithm").map(str::trim) {
        None | Some("") | Some("HMAC-SHA-1") => "SHA1",
        Some("HMAC-SHA-256") => "SHA256",
        Some("HMAC-SHA-512") => "SHA512",
        Some(_) => return Err(Invalid::Settings),
    };
    check(digits, period, algorithm)?;
    Ok(link(e.get_title().unwrap_or(""), secret, digits, period, algorithm, None))
}

/// Settings the code can be computed with (more than 9 digits overflow).
fn check(digits: u32, period: u64, algorithm: &str) -> Result<(), Invalid> {
    let ok = (1..=9).contains(&digits) && period > 0 && matches!(algorithm, "SHA1" | "SHA256" | "SHA512");
    if ok { Ok(()) } else { Err(Invalid::Settings) }
}

/// Base32 as the keepass crate reads it: upper case, padded, no spaces.
fn base32_secret(text: &str) -> Result<Zeroizing<String>, Invalid> {
    let mut s = Zeroizing::new(String::with_capacity(text.len() + 8));
    for ch in text.chars() {
        match ch.to_ascii_uppercase() {
            c @ ('A'..='Z' | '2'..='7') => s.push(c),
            ' ' | '-' | '=' => {}
            _ => return Err(Invalid::Secret),
        }
    }
    if s.is_empty() || base32::decode(base32::Alphabet::Rfc4648 { padding: false }, &s).is_none_or(|b| b.is_empty()) {
        return Err(Invalid::Secret);
    }
    while !s.len().is_multiple_of(8) {
        s.push('=');
    }
    Ok(s)
}

fn link(label: &str, secret: &str, digits: u32, period: u64, algorithm: &str, issuer: Option<&str>) -> Zeroizing<String> {
    let mut url = Url::parse("otpauth://totp/").expect("a valid link");
    url.set_path(label);
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("secret", secret);
        if let Some(i) = issuer {
            q.append_pair("issuer", i);
        }
        q.append_pair("period", &period.to_string());
        q.append_pair("digits", &digits.to_string());
        q.append_pair("algorithm", algorithm);
    }
    Zeroizing::new(url.into())
}

fn percent_decoded(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={}", s.replace('+', "%2B")).as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 6238's test secret ("12345678901234567890") and its SHA-1 codes.
    const RFC: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

    #[test]
    fn computes_the_rfc_codes() {
        let link = format!("otpauth://totp/Example:alex?secret={RFC}&digits=8");
        assert_eq!(code_at(&link, 59).unwrap().code.as_str(), "94287082");
        assert_eq!(code_at(&link, 1111111109).unwrap().code.as_str(), "07081804");
        let c = code_at(&link, 1111111109).unwrap();
        assert_eq!((c.left, c.period), (1, 30));
    }

    #[test]
    fn a_link_without_digits_gives_six() {
        let link = format!("otpauth://totp/Example?secret={RFC}");
        assert_eq!(code_at(&link, 59).unwrap().code.as_str(), "287082");
    }

    #[test]
    fn a_bare_secret_in_any_case_with_spaces_works() {
        let typed = RFC.to_lowercase().as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join(" ");
        assert_eq!(code_at(&typed, 59).unwrap().code.as_str(), "287082");
        let link = normalize(&typed, "Bank").unwrap();
        assert!(link.starts_with("otpauth://totp/Bank?secret=GEZD"), "{}", link.as_str());
        assert!(link.ends_with("&period=30&digits=6&algorithm=SHA1"));
    }

    #[test]
    fn keeps_the_label_and_issuer() {
        let link = normalize(&format!("otpauth://totp/Example%20Co:alex%40example.org?secret={RFC}&issuer=Example%20Co"), "").unwrap();
        let again = normalize(&link, "").unwrap();
        assert_eq!(link, again);
        assert!(link.contains("issuer=Example+Co"));
        assert!(link.starts_with("otpauth://totp/Example%20Co:alex@example.org?"));
    }

    #[test]
    fn rejects_what_it_cant_compute() {
        assert_eq!(normalize(&format!("otpauth://hotp/x?secret={RFC}&counter=1"), "").unwrap_err(), Invalid::NotTimeBased);
        assert_eq!(normalize(&format!("otpauth://totp/x?secret={RFC}&encoder=steam"), "").unwrap_err(), Invalid::Unsupported);
        assert_eq!(normalize("otpauth://totp/x?digits=6", "").unwrap_err(), Invalid::Secret);
        assert_eq!(normalize("not base32!", "").unwrap_err(), Invalid::Secret);
        assert_eq!(normalize(&format!("otpauth://totp/x?secret={RFC}&digits=12"), "").unwrap_err(), Invalid::Settings);
        assert_eq!(normalize(&format!("otpauth://totp/x?secret={RFC}&period=0"), "").unwrap_err(), Invalid::Settings);
        assert_eq!(normalize(&format!("otpauth://totp/x?secret={RFC}&algorithm=MD5"), "").unwrap_err(), Invalid::Settings);
    }

    #[test]
    fn reads_keepass_time_otp_fields() {
        let mut db = keepass::Database::new();
        let mut root = db.root_mut();
        let mut e = root.add_entry();
        e.set_protected("TimeOtp-Secret", "12345678901234567890");
        e.set_unprotected("TimeOtp-Length", "8");
        let id = e.id();
        let link = stored(&db.entry(id).unwrap()).unwrap().unwrap();
        assert_eq!(code_at(&link, 59).unwrap().code.as_str(), "94287082");

        let mut e = db.entry_mut(id).unwrap();
        e.fields.remove("TimeOtp-Secret");
        e.set_protected("TimeOtp-Secret-Hex", "3132333435363738393031323334353637383930");
        e.set_unprotected("TimeOtp-Algorithm", "HMAC-SHA-1");
        let link = stored(&db.entry(id).unwrap()).unwrap().unwrap();
        assert_eq!(code_at(&link, 1111111109).unwrap().code.as_str(), "07081804");

        let mut e = db.entry_mut(id).unwrap();
        e.fields.remove("TimeOtp-Secret-Hex");
        e.set_protected("TimeOtp-Secret-Base64", "MTIzNDU2Nzg5MDEyMzQ1Njc4OTA=");
        let link = stored(&db.entry(id).unwrap()).unwrap().unwrap();
        assert_eq!(code_at(&link, 59).unwrap().code.as_str(), "94287082");
    }

    #[test]
    fn an_entry_without_one_has_none() {
        let mut db = keepass::Database::new();
        let id = db.root_mut().add_entry().id();
        assert!(stored(&db.entry(id).unwrap()).is_none());
    }

    #[test]
    fn groups_the_digits() {
        assert_eq!(group("123456").as_str(), "123 456");
        assert_eq!(group("12345678").as_str(), "1234 5678");
        assert_eq!(group("12345").as_str(), "12345");
    }
}
