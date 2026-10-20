//! Vendor custom extensions (`$defs.CustomExtension`).

use serde::{Deserialize, Serialize};

/// Vendor-specific attributes for extensibility.
///
/// `vendor_name` is a free-form string and `data` is an opaque JSON document.
/// This crate carries extensions through parse and serialization but does not
/// interpret any of them — nothing outside the Ossie core spec affects planning.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomExtension {
    pub vendor_name: String,
    pub data: String,
}
