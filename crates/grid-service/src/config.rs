//! Schema types for generic per-service configuration.
//!
//! Services declare their settings via [`ConfigField`] descriptors so the UI
//! can render appropriate widgets without service-specific code.

/// A single configurable field exposed by a service.
#[derive(Debug, Clone)]
pub struct ConfigField {
    /// Machine-readable key used in the persisted JSON (e.g. `"self_validate"`).
    pub key: &'static str,
    /// Human-readable label shown in the UI.
    pub label: &'static str,
    /// Tooltip / help text.
    pub description: &'static str,
    /// Type and default value.
    pub field_type: ConfigFieldType,
}

/// The value type and default for a [`ConfigField`].
#[derive(Debug, Clone)]
pub enum ConfigFieldType {
    Bool {
        default: bool,
    },
    String {
        default: &'static str,
    },
    U64 {
        default: u64,
        min: Option<u64>,
        max: Option<u64>,
    },
}
