use super::*;

thread_local! {
    pub(crate) static CURRENT_CONFIG_FILE: RefCell<String> = RefCell::new(String::new());
}

/// Get the current config file path being parsed.
pub fn current_config_file() -> String {
    CURRENT_CONFIG_FILE.with(|f| f.borrow().clone())
}

/// Set the current config file path.
pub(crate) fn set_current_config_file(path: &str) {
    CURRENT_CONFIG_FILE.with(|f| *f.borrow_mut() = path.to_string());
}

/// Check if a config-level condition result is truthy
pub(crate) fn is_truthy_config(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty() && s != "0"
}
