use rand::Rng;
/// Generates a random string of given length.
///
/// # Examples
///
/// ```
/// use quilt_rs::random_string;
///
/// let s = random_string(10);
/// assert_eq!(s.len(), 10);
/// ```
pub fn random_string(length: usize) -> String {
    let mut rng = rand::thread_rng();
    let chars: Vec<char> = (0..length)
        .map(|_| rng.gen_range('a'..'z') as char)
        .collect();
    chars.into_iter().collect()
}
