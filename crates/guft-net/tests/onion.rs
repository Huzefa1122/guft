use guft_core::payload::check_onion;
use guft_net::tor::onion_address;

#[test]
fn address_is_valid_deterministic_and_seed_specific() {
    let a = onion_address(&[1u8; 32]);
    assert_eq!(a.len(), 62);
    assert!(check_onion(&a).is_ok(), "{a}");
    assert_eq!(a, onion_address(&[1u8; 32]));
    assert_ne!(a, onion_address(&[2u8; 32]));
    // The v3 version byte makes every address end in 'd' before ".onion".
    assert!(a.ends_with("d.onion"));
}
