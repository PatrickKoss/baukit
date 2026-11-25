#![no_main]

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use baukit_integrations::oauth_state::{
    BrowserBinding, OAUTH_BINDING_COOKIE, OAuthState, TOKEN_CHARS, binding_from_cookie_header,
};
use libfuzzer_sys::fuzz_target;

fn canonical(text: &str) -> bool {
    URL_SAFE_NO_PAD
        .decode(text)
        .is_ok_and(|bytes| URL_SAFE_NO_PAD.encode(bytes) == text)
}

fn exact_cookie(header: &str) -> Option<&str> {
    header
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == OAUTH_BINDING_COOKIE)
        .map(|(_, value)| value)
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let parsed = OAuthState::parse(text);
    assert_eq!(parsed.is_ok(), text.len() == TOKEN_CHARS && canonical(text));
    if let Ok(state) = parsed {
        assert_eq!(state.expose(), text);
    }
    let value = binding_from_cookie_header(text);
    assert_eq!(value, exact_cookie(text));
    if let Some(value) = value
        && let Ok(binding) = BrowserBinding::parse(value)
    {
        assert_eq!(binding.expose(), value);
        assert!(canonical(value));
        assert!(binding.hash().matches(&binding.clone().hash()));
    }
});
