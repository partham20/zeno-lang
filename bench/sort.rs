fn quicksort(xs: &mut Vec<i64>, low: i64, high: i64) {
    let (mut lo, mut hi) = (low, high);
    while lo < hi {
        let pivot = xs[((lo + hi) / 2) as usize];
        let (mut i, mut j) = (lo, hi);
        while i <= j {
            while xs[i as usize] < pivot { i += 1; }
            while xs[j as usize] > pivot { j -= 1; }
            if i <= j { xs.swap(i as usize, j as usize); i += 1; j -= 1; }
        }
        if j - lo < hi - i { quicksort(xs, lo, j); lo = i; } else { quicksort(xs, i, hi); hi = j; }
    }
}
fn main() {
    let n: i64 = std::env::args().nth(1).map(|a| a.parse().unwrap()).unwrap_or(1000000);
    let mut xs: Vec<i64> = Vec::new();
    let mut seed: i64 = 42;
    for _ in 0..n { seed = (seed * 1103515245 + 12345) % 2147483648; xs.push(seed); }
    quicksort(&mut xs, 0, n - 1);
    let ok = (1..n as usize).all(|k| xs[k - 1] <= xs[k]);
    println!("{} {} {} {}", ok, xs[0], xs[(n / 2) as usize], xs[(n - 1) as usize]);
}
