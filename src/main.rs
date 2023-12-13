use rand::Rng;

fn random_string(length: usize) -> String {
    let mut rng = rand::thread_rng();
    let chars: Vec<char> = (0..length)
        .map(|_| rng.gen_range('a'..'z') as char)
        .collect();
    chars.into_iter().collect()
}

 
fn main() {
    let s = random_string(10);
    println!("Hello, {}!", s);
}
