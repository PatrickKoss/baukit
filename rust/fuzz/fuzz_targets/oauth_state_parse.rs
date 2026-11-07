#![no_main]

use baukit_integrations::oauth_state::{
    BrowserBinding, OAUTH_BINDING_COOKIE, OAuthState, TOKEN_CHARS, binding_from_cookie_header,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(state) = OAuthState::parse(text) {
        assert_eq!(state.expose(), text);
        assert_eq!(text.len(), TOKEN_CHARS);
    }
    if let Some(value) = binding_from_cookie_header(text) {
        assert!(text.contains(OAUTH_BINDING_COOKIE));
        if let Ok(binding) = BrowserBinding::parse(value) {
            assert_eq!(binding.expose(), value);
            assert!(binding.hash().matches(&binding.clone().hash()));
        }
    }
});
