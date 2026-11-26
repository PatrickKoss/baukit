#![no_main]

use baukit_push::{DeviceToken, internals::parse_tickets};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (u8, &[u8])| {
    let (count, body) = input;
    let tokens = (0..count % 8)
        .filter_map(|index| DeviceToken::new(format!("token-{index}")).ok())
        .collect::<Vec<_>>();
    if let Ok((outcomes, pending)) = parse_tickets(&tokens, body) {
        assert_eq!(outcomes.len() + pending.len(), tokens.len());
    }
});
