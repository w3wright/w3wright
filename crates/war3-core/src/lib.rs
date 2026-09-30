//! Shared core types for w3wright.
//!
//! Contains the types every format crate needs: [`FourCC`], [`Vec3`], the error
//! and diagnostic machinery, and the injectable [`AssetSource`].
//!
//! Deliberately free of any format-specific code and of any UI, container or
//! rendering dependency, so the same crate compiles for native targets and for
//! WASM.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod assets;
pub mod diag;
pub mod error;
pub mod fourcc;
pub mod math;

pub use assets::{AssetSource, EmptyAssetSource, FileAssetSource, MemoryAssetSource};
pub use diag::{Diagnostic, DiagnosticCode, Diagnostics, Severity};
pub use error::{Error, ParseError, Result};
pub use fourcc::FourCC;
pub use math::Vec3;
