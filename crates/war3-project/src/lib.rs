//! Source project: a map as a directory.
//!
//! Implements the design recorded in `docs/` (ADR-0021): **every archive member
//! is either textified or kept verbatim** — one or the other, never dropped.
//! A member with no text form yet is either decoded and stored as content
//! (`[binary]`) or stored as its original block (`[raw]`).
//!
//! ```text
//! war3 map extract map.w3x project/
//! war3 build project/ --out map.w3x
//! ```
//!
//! Three properties are load-bearing:
//!
//! - **The manifest is the authority.** `build` never re-derives where a member
//!   lives, so a file may be renamed by hand as long as the manifest is updated.
//! - **`build` verifies itself.** Every build reads its own output back and
//!   compares each member with the file it came from, because "it wrote without
//!   an error" is not the same as "a reader gets the same bytes".
//! - **Textifying is proved, not assumed.** Before a member is written as text,
//!   its text form is parsed back and compared with the original bytes; a member
//!   that does not come back identical stays binary. A serialiser that is right
//!   about 188 maps and quietly wrong about 2 is worse than no serialiser.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod codecs;
pub mod disposition;
pub mod config;
pub mod doodads_text;
pub mod ini;
pub mod layout;
pub mod objects_text;
pub mod project;
pub mod raw;
pub mod script_text;
pub mod units_text;
pub mod w3i_text;

pub use config::{Config, Members};
pub use disposition::Disposition;
pub use project::{
    build, extract, validate, BuildReport, ExtractReport, ValidateReport, PREFIX_PATH,
};
