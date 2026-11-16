fn main() {
    let n: i64 = std::env::args().nth(1).map(|a| a.parse().unwrap()).unwrap_or(100000000);
    let mut total: i64 = 0;
    let mut i: i64 = 0;
    while i < n { total += (i * i) % 7; i += 1; }
    println!("{}", total);
}
