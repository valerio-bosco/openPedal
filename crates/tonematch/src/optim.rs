//! Nelder-Mead in the unit cube [0,1]^n, with restarts.

/// Result of a minimisation.
#[derive(Debug, Clone)]
pub struct Minimum {
    pub x: Vec<f64>,
    pub value: f64,
    pub evals: usize,
}

/// Distance outside the cube is penalised; the objective only ever sees clamped points.
fn eval(f: &mut dyn FnMut(&[f64]) -> f64, x: &[f64], evals: &mut usize) -> f64 {
    let clamped: Vec<f64> = x.iter().map(|v| v.clamp(0.0, 1.0)).collect();
    let outside: f64 = x.iter().zip(&clamped).map(|(a, b)| (a - b).abs()).sum();
    *evals += 1;
    f(&clamped) + 100.0 * outside
}

/// One Nelder-Mead run of at most `budget` evaluations starting with a simplex of edge `step`.
pub fn nelder_mead(f: &mut dyn FnMut(&[f64]) -> f64, x0: &[f64], step: f64, budget: usize) -> Minimum {
    let n = x0.len();
    let mut evals = 0usize;
    let mut simplex: Vec<(Vec<f64>, f64)> = Vec::with_capacity(n + 1);
    let start: Vec<f64> = x0.iter().map(|v| v.clamp(0.0, 1.0)).collect();
    simplex.push((start.clone(), eval(f, &start, &mut evals)));
    for i in 0..n {
        let mut p = start.clone();
        // Step inwards when the start sits on the upper edge.
        p[i] += if p[i] + step > 1.0 { -step } else { step };
        let v = eval(f, &p, &mut evals);
        simplex.push((p, v));
    }
    let (alpha, gamma, rho, sigma) = (1.0, 2.0, 0.5, 0.5);
    while evals < budget {
        simplex.sort_by(|a, b| a.1.total_cmp(&b.1));
        if (simplex[n].1 - simplex[0].1).abs() < 1e-9 && simplex.iter().all(|p| dist(&p.0, &simplex[0].0) < 1e-6) {
            break;
        }
        let centroid: Vec<f64> = (0..n).map(|j| simplex[..n].iter().map(|p| p.0[j]).sum::<f64>() / n as f64).collect();
        let along = |t: f64| -> Vec<f64> { (0..n).map(|j| centroid[j] + t * (simplex[n].0[j] - centroid[j])).collect() };
        let xr = along(-alpha);
        let fr = eval(f, &xr, &mut evals);
        if fr < simplex[0].1 {
            let xe = along(-gamma);
            let fe = eval(f, &xe, &mut evals);
            simplex[n] = if fe < fr { (xe, fe) } else { (xr, fr) };
        } else if fr < simplex[n - 1].1 {
            simplex[n] = (xr, fr);
        } else {
            let (xc, outside) = if fr < simplex[n].1 { (along(-rho), true) } else { (along(rho), false) };
            let fc = eval(f, &xc, &mut evals);
            let limit = if outside { fr } else { simplex[n].1 };
            if fc < limit {
                simplex[n] = (xc, fc);
            } else {
                let best = simplex[0].0.clone();
                for p in simplex.iter_mut().skip(1) {
                    for (v, b) in p.0.iter_mut().zip(&best) {
                        *v = b + sigma * (*v - b);
                    }
                    p.1 = eval(f, &p.0, &mut evals);
                }
            }
        }
    }
    simplex.sort_by(|a, b| a.1.total_cmp(&b.1));
    let (x, value) = simplex.swap_remove(0);
    Minimum { x: x.iter().map(|v| v.clamp(0.0, 1.0)).collect(), value, evals }
}

/// Nelder-Mead plus `restarts` fresh simplices (smaller each time) around the best point so far,
/// within a total budget of `budget` evaluations (about half of it for the first run).
pub fn minimize(f: &mut dyn FnMut(&[f64]) -> f64, x0: &[f64], budget: usize, restarts: usize) -> Minimum {
    let first = if restarts == 0 { budget } else { budget / 2 };
    let mut best = nelder_mead(f, x0, 0.15, first);
    let mut used = best.evals;
    for r in 0..restarts {
        let left = budget.saturating_sub(used);
        if left < x0.len() + 2 {
            break;
        }
        let share = left / (restarts - r);
        let run = nelder_mead(f, &best.x, 0.08 / (r as f64 + 1.0), share);
        used += run.evals;
        if run.value < best.value {
            best = Minimum { evals: 0, ..run };
        }
    }
    best.evals = used;
    best
}

fn dist(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max)
}
