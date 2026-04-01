//! Compilation passes acting on the HUGR program representation.
//!
//! <div class="warning">This crate is deprecated. Use [`tket::passes`](https://docs.rs/tket/latest/tket/passes/index.html) instead.</div>
#![allow(deprecated)]

#[deprecated(
    note = "`hugr-passes` is deprecated. Use tket::passes instead",
    since = "0.26.2"
)]
pub mod composable;
#[deprecated(
    note = "`hugr-passes` is deprecated. Use tket::passes instead",
    since = "0.26.2"
)]
pub mod const_fold;
#[deprecated(
    note = "`hugr-passes` is deprecated. Use tket::passes instead",
    since = "0.26.2"
)]
pub mod dataflow;
#[deprecated(
    note = "`hugr-passes` is deprecated. Use tket::passes instead",
    since = "0.26.2"
)]
pub mod dead_code;
#[deprecated(
    note = "`hugr-passes` is deprecated. Use tket::passes instead",
    since = "0.26.2"
)]
pub mod force_order;
#[deprecated(
    note = "`hugr-passes` is deprecated. Use tket::passes instead",
    since = "0.26.2"
)]
pub mod hash;
#[deprecated(
    note = "`hugr-passes` is deprecated. Use tket::passes instead",
    since = "0.26.2"
)]
pub mod inline_dfgs;
#[deprecated(
    note = "`hugr-passes` is deprecated. Use tket::passes instead",
    since = "0.26.2"
)]
pub mod inline_funcs;
#[deprecated(
    note = "`hugr-passes` is deprecated. Use tket::passes instead",
    since = "0.26.2"
)]
pub mod lower;
#[deprecated(
    note = "`hugr-passes` is deprecated. Use tket::passes instead",
    since = "0.26.2"
)]
pub mod nest_cfgs;
#[deprecated(
    note = "`hugr-passes` is deprecated. Use tket::passes instead",
    since = "0.26.2"
)]
pub mod non_local;
#[deprecated(
    note = "`hugr-passes` is deprecated. Use tket::passes instead",
    since = "0.26.2"
)]
pub mod normalize_cfgs;
#[deprecated(
    note = "`hugr-passes` is deprecated. Use tket::passes instead",
    since = "0.26.2"
)]
pub mod redundant_order_edges;
#[deprecated(
    note = "`hugr-passes` is deprecated. Use tket::passes instead",
    since = "0.26.2"
)]
pub mod replace_types;
#[deprecated(
    note = "`hugr-passes` is deprecated. Use tket::passes instead",
    since = "0.26.2"
)]
pub mod untuple;

mod dead_funcs;
mod half_node;
mod monomorphize;
#[cfg(test)]
mod utils;

// Main pass interfaces
pub use composable::{ComposablePass, InScope, PassScope};

// Pass re-exports
pub use dead_code::DeadCodeElimPass;
pub use dead_funcs::{RemoveDeadFuncsError, RemoveDeadFuncsPass};
pub use force_order::{force_order, force_order_by_key};
pub use inline_funcs::inline_acyclic;
pub use lower::{lower_ops, replace_many_ops};
pub use monomorphize::{MonomorphizePass, mangle_name};
#[expect(deprecated)]
pub use non_local::ensure_no_nonlocal_edges;
pub use replace_types::ReplaceTypes;
pub use untuple::UntuplePass;
