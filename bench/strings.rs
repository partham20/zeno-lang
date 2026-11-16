fn main() {
    let n: i64 = std::env::args().nth(1).map(|a| a.parse().unwrap()).unwrap_or(1_000_000);
    let parts: Vec<String> = (0..n).map(|i| i.to_string()).collect();
    let text = parts.join(",");
    let commas = text.chars().filter(|&c| c == ',').count();
    println!("{} {}", text.chars().count(), commas);
}
