pub mod cdp;
pub mod detect;
pub mod register;

pub use register::{register_one, register_with_retries, resolve_browser, RegisterOptions};
