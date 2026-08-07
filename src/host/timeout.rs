use std::time::Duration;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
pub const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(25);
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(15 * 60);
pub const SESSION_TIMEOUT: Duration = Duration::from_secs(60);
pub const TREE_NAVIGATION_TIMEOUT: Duration = Duration::from_secs(15 * 60);
