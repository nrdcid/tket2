//! Benchmarks for the tket crate.
#![allow(deprecated)]

mod benchmarks;

use criterion::criterion_main;

criterion_main! {
    benchmarks::hash::benches,
}
