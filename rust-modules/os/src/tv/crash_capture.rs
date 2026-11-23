//! The platform's native crash recorder. Telemetry owns consent and delivery.

use std::path::Path;

pub struct Start<'a> {
    pub dsn: &'a str,
    pub database: &'a Path,
    pub release: &'a str,
    pub environment: &'a str,
    pub build_id: &'a str,
    pub user: Option<&'a str>,
    pub max_breadcrumbs: usize,
}

pub struct Breadcrumb {
    pub stage: &'static str,
    pub playing: Option<bool>,
    pub version: Option<[u8; 3]>,
    pub display: Option<bool>,
    pub surface: Option<bool>,
}

pub struct Hooks {
    pub start: Option<fn(Start<'_>)>,
    pub stop: fn(),
    pub set_user: fn(Option<&str>),
    pub record_window: fn(Breadcrumb),
}

impl Hooks {
    pub const NONE: Self = Self {
        start: None,
        stop: || {},
        set_user: |_| {},
        record_window: |_| {},
    };
}

pub fn installed() -> &'static Hooks {
    &super::services().crash_capture
}
