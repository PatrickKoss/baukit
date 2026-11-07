#![no_main]

use libfuzzer_sys::fuzz_target;
use uuid::Uuid;

use {{ context.app_crate }}_api::SaveItemRequest;
use {{ context.app_crate }}_domain::Item;

fuzz_target!(|data: &[u8]| {
    let Ok(request) = serde_json::from_slice::<SaveItemRequest>(data) else {
        return;
    };
    if let Ok(item) = Item::new(Uuid::nil(), "fuzz-owner", request.name) {
        assert!(!item.name.is_empty());
        assert_eq!(item.name.trim(), item.name);
    }
});
