/// Validate that `value` is a syntactically valid hostname
/// (two or more dot-separated labels, each starting and ending with
/// an ASCII alphanumeric character, with hyphens allowed in the middle).
pub fn is_valid_hostname(value: &str) -> bool {
    let mut count = 0u32;
    for label in value.split('.') {
        if !is_valid_label(label) {
            return false;
        }
        count += 1;
    }
    count >= 2
}

fn is_valid_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    !bytes.is_empty()
        && bytes[0].is_ascii_alphanumeric()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && bytes.iter().all(|&b| b.is_ascii_alphanumeric() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_hostnames() {
        assert!(is_valid_hostname("example.com"));
        assert!(is_valid_hostname("sub.example.com"));
        assert!(is_valid_hostname("my-host.example.co.uk"));
        assert!(is_valid_hostname("a.b"));
        assert!(is_valid_hostname("123.456"));
        assert!(is_valid_hostname("a-1.b-2.c-3"));
    }

    #[test]
    fn rejects_single_label() {
        assert!(!is_valid_hostname("localhost"));
        assert!(!is_valid_hostname("example"));
    }

    #[test]
    fn rejects_empty_and_dot_only() {
        assert!(!is_valid_hostname(""));
        assert!(!is_valid_hostname("."));
        assert!(!is_valid_hostname(".."));
    }

    #[test]
    fn rejects_leading_trailing_hyphen() {
        assert!(!is_valid_hostname("-example.com"));
        assert!(!is_valid_hostname("example-.com"));
        assert!(!is_valid_hostname("example.-com"));
        assert!(!is_valid_hostname("example.com-"));
    }

    #[test]
    fn rejects_invalid_characters() {
        assert!(!is_valid_hostname("exam ple.com"));
        assert!(!is_valid_hostname("exam_ple.com"));
        assert!(!is_valid_hostname("example!.com"));
    }

    #[test]
    fn rejects_empty_labels() {
        assert!(!is_valid_hostname(".example.com"));
        assert!(!is_valid_hostname("example..com"));
        assert!(!is_valid_hostname("example.com."));
    }
}
