//! All L1 related primitives.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc as std;

/// L1 contracts Abi.
pub mod abi;
