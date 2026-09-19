//! Independent game-script and CWT syntax, without game semantics or file I/O.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod cwt;
pub mod script;
mod source;
pub use source::{ErrorKind, Span, SyntaxError};

/// Maximum number of nested container or conditional-region bodies.
pub const MAX_NESTING_DEPTH: usize = 1000;
