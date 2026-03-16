/// Returns a static daemon bootstrap message.
pub fn bootstrap_message() -> &'static str {
    "arroba daemon bootstrap"
}

#[cfg(test)]
mod tests {
    use super::bootstrap_message;

    #[test]
    fn bootstrap_message_is_stable() {
        assert_eq!(bootstrap_message(), "arroba daemon bootstrap");
    }
}
