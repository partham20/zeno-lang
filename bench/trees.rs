struct Node { left: Option<Box<Node>>, right: Option<Box<Node>> }
fn make(d: i64) -> Box<Node> {
    if d == 0 { Box::new(Node { left: None, right: None }) }
    else { Box::new(Node { left: Some(make(d - 1)), right: Some(make(d - 1)) }) }
}
fn check(n: &Option<Box<Node>>) -> i64 { match n { None => 0, Some(n) => 1 + check(&n.left) + check(&n.right) } }
fn main() {
    let deepest: i64 = std::env::args().nth(1).map(|a| a.parse().unwrap()).unwrap_or(16);
    let long_lived = Some(make(deepest));
    let mut depth = 4;
    while depth <= deepest {
        let iterations: i64 = 1 << (deepest - depth + 4);
        let mut total = 0;
        for _ in 0..iterations { let t = Some(make(depth)); total += check(&t); }
        println!("{} trees of depth {} check {}", iterations, depth, total);
        depth += 2;
    }
    println!("long lived tree of depth {} check {}", deepest, check(&long_lived));
}
