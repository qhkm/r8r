use r8r::cli::init::{detect_env_key, detect_shell};

#[test]
fn test_detect_shell() {
    let result = detect_shell();
    // Should always return a result on any system
    assert!(result.detected);
    assert!(!result.detail.is_empty());
}

#[test]
fn test_detect_env_key_missing() {
    // Use a key that certainly doesn't exist
    let result = detect_env_key("R8R_TEST_NONEXISTENT_KEY_12345");
    assert!(!result.detected);
}

#[test]
fn test_detect_env_key_present() {
    std::env::set_var("R8R_TEST_INIT_KEY", "test-value");
    let result = detect_env_key("R8R_TEST_INIT_KEY");
    assert!(result.detected);
    std::env::remove_var("R8R_TEST_INIT_KEY");
}
