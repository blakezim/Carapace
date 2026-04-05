//! Channel adapters — concrete implementations that call real messaging tools.
//!
//! Each adapter wraps an external tool or daemon and provides typed methods
//! for the channel operations the Carapace gateway exposes.

pub mod gmail;
pub mod imsg;
