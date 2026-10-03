use subtle::ConstantTimeEq;

/// Compares secret bytes without short-circuiting on a differing byte.
///
/// Runtime depends on the lengths, which are not concealed. Unequal lengths
/// return `false`; equal-length inputs use `subtle`'s constant-time comparison.
/// Pass strings with `.as_bytes()`, or compare fixed-size digests when secret
/// lengths must also stay private.
#[must_use]
pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    bool::from(left.ct_eq(right))
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn equal_bytes_including_empty_and_binary_inputs_match() {
        for bytes in [b"".as_slice(), b"secret", &[0, 255, 128, 0]] {
            assert!(constant_time_eq(bytes, bytes));
        }
    }

    #[test]
    fn each_byte_and_length_difference_is_rejected() {
        let secret = [0, 1, 128, 255];
        for index in 0..secret.len() {
            let mut changed = secret;
            changed[index] ^= 1;
            assert!(!constant_time_eq(&secret, &changed));
        }
        assert!(!constant_time_eq(&secret, &secret[..3]));
        assert!(!constant_time_eq(&secret[..3], &secret));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(!constant_time_eq(b"x", b""));
    }
}
