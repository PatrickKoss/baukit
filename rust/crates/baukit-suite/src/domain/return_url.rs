use super::SuiteDataError;
use url::Url;

pub(super) const DEV_SCHEME_SUFFIX: &str = "-dev";

/// # Errors
///
/// `SuiteDataError::ReturnUrlInvalid` when either URL does not parse, either carries
/// credentials, the origin has a path, query or fragment, or `value` is neither
/// `<origin>/suite/linked` nor `<own_scheme>://suite/linked`. With `allow_dev_scheme`, the
/// `<own_scheme>-dev` scheme of the template's `dev` build variant is accepted too.
pub fn validate_return_url(
    value: &str,
    own_web_origin: &str,
    own_scheme: &str,
    allow_dev_scheme: bool,
) -> Result<(), SuiteDataError> {
    let invalid = || SuiteDataError::ReturnUrlInvalid;
    let origin = Url::parse(own_web_origin).map_err(|_| invalid())?;
    let url = Url::parse(value).map_err(|_| invalid())?;
    if !url.username().is_empty()
        || url.password().is_some()
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.path() != "/"
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return Err(invalid());
    }
    let web = matches!(url.scheme(), "https" | "http")
        && url.origin() == origin.origin()
        && url.path() == "/suite/linked";
    let native = is_own_native_scheme(url.scheme(), own_scheme, allow_dev_scheme)
        && url.host_str() == Some("suite")
        && url.port().is_none()
        && url.path() == "/linked";
    if web || native {
        Ok(())
    } else {
        Err(invalid())
    }
}

/// Web schemes never count as a native app scheme, even if a registry names one.
const NON_NATIVE_SCHEMES: [&str; 3] = ["http", "https", "javascript"];

pub(crate) fn is_own_native_scheme(scheme: &str, own_scheme: &str, allow_dev_scheme: bool) -> bool {
    if NON_NATIVE_SCHEMES.contains(&scheme) {
        return false;
    }
    scheme == own_scheme
        || allow_dev_scheme
            && scheme
                .strip_suffix(DEV_SCHEME_SUFFIX)
                .is_some_and(|base| base == own_scheme)
}

#[cfg(kani)]
mod proofs {
    use super::*;
    use crate::domain::signature_timestamp_valid;

    const MAX_SCHEME_BYTES: usize = 7;

    fn ascii<const N: usize>(len: usize) -> ([u8; N], usize) {
        let bytes: [u8; N] = kani::any();
        kani::assume(len <= N);
        for byte in &bytes[..len] {
            kani::assume(byte.is_ascii_lowercase() || *byte == b'-');
        }
        (bytes, len)
    }

    /// K-15
    /// Bounds: every `i64` timestamp and now, no loops.
    #[kani::proof]
    fn signature_window_is_symmetric_without_overflow() {
        let timestamp: i64 = kani::any();
        let now: i64 = kani::any();
        let difference = i128::from(timestamp) - i128::from(now);
        assert_eq!(
            signature_timestamp_valid(timestamp, now),
            (-300..=300).contains(&difference)
        );
    }

    /// K-16
    /// Bounds: ASCII schemes of 0 to 7 bytes, own scheme of 0 to 3 bytes, unwind 8.
    #[kani::proof]
    #[kani::unwind(8)]
    fn native_scheme_is_own_or_opted_in_dev() {
        let (scheme, scheme_len) = ascii::<MAX_SCHEME_BYTES>(kani::any());
        let (own, own_len) = ascii::<3>(kani::any());
        let allow_dev: bool = kani::any();
        let (Ok(scheme), Ok(own)) = (
            std::str::from_utf8(&scheme[..scheme_len]),
            std::str::from_utf8(&own[..own_len]),
        ) else {
            return;
        };
        let expected = !NON_NATIVE_SCHEMES.contains(&scheme)
            && (scheme == own
                || allow_dev
                    && scheme.len() == own.len() + 4
                    && scheme.starts_with(own)
                    && scheme.ends_with("-dev"));
        assert_eq!(is_own_native_scheme(scheme, own, allow_dev), expected);
    }
}
