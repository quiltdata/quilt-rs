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

// https://doc.rust-lang.org/book/ch11-01-writing-tests.html

#[derive(Debug)]
pub struct Rectangle {
    pub width: u32,
    pub height: u32,
}

impl Rectangle {
    pub fn can_hold(&self, other: &Rectangle) -> bool {
        self.width > other.width && self.height > other.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn larger_can_hold_smaller() {
        let larger = Rectangle {
            width: 8,
            height: 7,
        };
        let smaller = Rectangle {
            width: 5,
            height: 1,
        };

        assert!(smaller.can_hold(&larger));
    }
}
