//! Channel adapters — concrete implementations that call real messaging tools.
//!
//! Each adapter wraps a CLI tool (e.g. `imsg`) and provides typed methods
//! for sending messages, listing chats, and querying history.
//! A `ChannelAdapter` trait will be introduced in Phase 6 when a second
//! channel is added.

pub mod imsg;
