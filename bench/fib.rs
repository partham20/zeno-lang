fn fib(n: i64) -> i64 { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }
fn main() { println!("{}", fib(std::env::args().nth(1).map(|a| a.parse().unwrap()).unwrap_or(32))); }
