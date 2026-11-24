use openpedal_tonematch::optim::minimize;

fn quadratic(x: &[f64]) -> f64 {
    x.iter().enumerate().map(|(i, v)| (v - 0.2 - 0.15 * i as f64).powi(2)).sum()
}

/// Rosenbrock with the unit cube mapped to [-2, 2]; the minimum (1, ..., 1) is at 0.75.
fn rosenbrock(x: &[f64]) -> f64 {
    let p: Vec<f64> = x.iter().map(|u| u * 4.0 - 2.0).collect();
    p.windows(2).map(|w| 100.0 * (w[1] - w[0] * w[0]).powi(2) + (1.0 - w[0]).powi(2)).sum()
}

#[test]
fn minimises_a_quadratic_in_2_to_4_dims() {
    for n in 2..=4 {
        let m = minimize(&mut quadratic, &vec![0.9; n], 600, 2);
        assert!(m.value < 1e-6, "{n} dims: {}", m.value);
        for (i, v) in m.x.iter().enumerate() {
            assert!((v - 0.2 - 0.15 * i as f64).abs() < 1e-3);
        }
    }
}

#[test]
fn minimises_rosenbrock_in_2_to_4_dims() {
    for n in 2..=4 {
        let m = minimize(&mut rosenbrock, &vec![0.4; n], 6000, 2);
        assert!(m.value < 0.05, "{n} dims: {}", m.value);
        assert!(m.evals <= 6000 + 10);
    }
}

#[test]
fn stays_in_the_box_and_finds_a_boundary_minimum() {
    // The unconstrained minimum is at x = 2: the best point in the box is the edge.
    let mut seen_outside = false;
    let mut f = |x: &[f64]| {
        seen_outside |= x.iter().any(|v| !(0.0..=1.0).contains(v));
        (x[0] - 2.0).powi(2) + (x[1] + 1.0).powi(2)
    };
    let m = minimize(&mut f, &[0.5, 0.5], 300, 2);
    assert!(!seen_outside);
    assert!(m.x[0] > 0.999 && m.x[1] < 0.001, "{:?}", m.x);
}
