/// Checks a Redis key namespace; the empty string means no namespace.
///
/// # Errors
///
/// A static reason when `namespace` contains whitespace, a Redis glob
/// character (`*`, `?`, `[`, `]`), or a non-empty value lacks the trailing `:`.
pub fn validate_redis_key_namespace(namespace: &str) -> Result<(), &'static str> {
    if namespace.is_empty() {
        return Ok(());
    }
    if namespace.chars().any(char::is_whitespace) {
        return Err("must not contain whitespace");
    }
    if namespace.contains(['*', '?', '[', ']']) {
        return Err("must not contain Redis glob characters");
    }
    if !namespace.ends_with(':') {
        return Err("must end with `:`");
    }
    Ok(())
}
