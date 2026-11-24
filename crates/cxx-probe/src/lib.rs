//! Spike: proves C++17 + Eigen links into the Rust core (docs/spikes/ios-cxx.md).

extern "C" {
    fn cxx_probe(n: i32) -> f32;
}

/// Sum of the top-left n x n block of (ones(64,64) * 2*I): `2 * n * n`, computed by Eigen on a thread.
pub fn probe(n: i32) -> f32 {
    // SAFETY: pure function of its argument, no pointers.
    unsafe { cxx_probe(n) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn eigen_matrix_product_on_a_thread() {
        assert_eq!(super::probe(8), 128.0);
        assert_eq!(super::probe(64), 8192.0);
    }
}
