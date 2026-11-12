//! A single-code-tool Android agent. Raw device responses stay outside model context.

pub mod agent;
pub mod api;
pub mod bootstrap;
pub mod executor;
pub mod logging;
pub mod model;
pub mod worker;

/// UTF-8-safe prefix, measured in bytes rather than characters or estimated tokens.
pub fn prefix(text: &str, bytes: usize) -> &str {
    let mut end = bytes.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
