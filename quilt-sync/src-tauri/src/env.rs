/// Environment variable handling for both development and production builds.
///
/// This module provides a unified interface for accessing environment variables:
/// - Development: Uses dotenv to load .env files + runtime env::var
/// - Production: Uses build-time option_env! for embedded values
use std::sync::Once;

static INIT: Once = Once::new();

/// Initialize environment loading (dotenv for development).
/// This should be called once at application startup.
pub fn init() {
    INIT.call_once(|| {
        #[cfg(debug_assertions)]
        {
            if let Err(e) = dotenv::dotenv() {
                // It's okay if .env file doesn't exist in development
                eprintln!("Note: .env file not found or invalid: {}", e);
            }
        }
    });
}

/// Get an environment variable value, trying runtime first, then build-time.
///
/// This function:
/// 1. First tries to get the value from runtime environment (env::var)
/// 2. Falls back to build-time embedded value (option_env!)
///
/// This allows:
/// - Development: Use .env files loaded by dotenv
/// - Production: Use values embedded at build time
///
/// Note: Build-time fallbacks must be explicitly added in the match statement
/// because option_env! requires string literals, not variables.
pub fn get_var(key: &str) -> Option<String> {
    // First try runtime environment variable
    if let Ok(value) = std::env::var(key) {
        if !value.is_empty() {
            return Some(value);
        }
    }

    // Fall back to build-time environment variable
    // Each variable must be explicitly listed because option_env!
    // requires string literals at compile time
    match key {
        "SENTRY_DSN" => option_env!("SENTRY_DSN").map(|s| s.to_string()),
        "MIXPANEL_PROJECT_TOKEN" => option_env!("MIXPANEL_PROJECT_TOKEN").map(|s| s.to_string()),
        "MIXPANEL_API_SECRET" => option_env!("MIXPANEL_API_SECRET").map(|s| s.to_string()),
        // Add more build-time env vars here as needed:
        // "DATABASE_URL" => option_env!("DATABASE_URL").map(|s| s.to_string()),
        // "API_KEY" => option_env!("API_KEY").map(|s| s.to_string()),
        _ => None,
    }
}

/// Get the SENTRY_DSN environment variable as a string.
pub fn sentry_dsn() -> Option<String> {
    get_var("SENTRY_DSN")
}

/// Get the MIXPANEL_PROJECT_TOKEN environment variable as a string.
pub fn mixpanel_project_token() -> Option<String> {
    get_var("MIXPANEL_PROJECT_TOKEN")
}

/// Get the MIXPANEL_API_SECRET environment variable as a string.
pub fn mixpanel_api_secret() -> Option<String> {
    get_var("MIXPANEL_API_SECRET")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_init() {
        // Should not panic
        init();
        init(); // Second call should be safe (Once)
    }

    #[test]
    fn test_get_var_with_runtime_env() {
        // Set a runtime env var
        unsafe {
            std::env::set_var("TEST_VAR", "runtime_value");
        }

        // Should get the runtime value
        assert_eq!(get_var("TEST_VAR"), Some("runtime_value".to_string()));

        // Clean up
        unsafe {
            std::env::remove_var("TEST_VAR");
        }
    }

    #[test]
    fn test_get_var_empty_value() {
        // Set empty value
        unsafe {
            std::env::set_var("EMPTY_VAR", "");
        }

        // Should return None for empty values
        assert_eq!(get_var("EMPTY_VAR"), None);

        // Clean up
        unsafe {
            std::env::remove_var("EMPTY_VAR");
        }
    }

    #[test]
    fn test_get_var_nonexistent() {
        // Should return None for non-existent vars
        assert_eq!(get_var("NONEXISTENT_VAR_12345"), None);
    }
}
