// SPDX-License-Identifier: MPL-2.0
//! Read-only access to UE5 IoStore containers (`.utoc` + `.ucas`) for content
//! the user is authorized to convert (ADR 0013). Nothing here ships in the
//! player; extracted data stays in a local cache outside the repository.
#![forbid(unsafe_code)]
pub mod graph;
pub mod iostore;
pub mod level;
pub mod props;
pub mod reflect;
pub mod usmap;
pub mod zen;
