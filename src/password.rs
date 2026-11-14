use rand::seq::SliceRandom;
use rand::Rng;

pub const SPECIAL: &str = "!@#$%^&*()-_=+[]{}|;:,.<>?/~";

/// Generate a strong password with the same guarantees as the Python version.
pub fn generate_password(length: usize, min_special: usize, min_digits: usize) -> String {
    let length = length.max(8);
    let lower = "abcdefghijklmnopqrstuvwxyz";
    let upper = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let digits = "0123456789";
    let mut rng = rand::thread_rng();
    let mut chars: Vec<char> = Vec::with_capacity(length);

    chars.push(random_from(lower, &mut rng));
    chars.push(random_from(upper, &mut rng));
    for _ in 0..min_digits.max(1) {
        chars.push(random_from(digits, &mut rng));
    }
    for _ in 0..min_special.max(1) {
        chars.push(random_from(SPECIAL, &mut rng));
    }
    let pool = format!("{lower}{upper}{digits}{SPECIAL}");
    while chars.len() < length {
        chars.push(random_from(&pool, &mut rng));
    }
    chars.truncate(length);
    chars.shuffle(&mut rng);
    chars.into_iter().collect()
}

fn random_from(set: &str, rng: &mut impl Rng) -> char {
    let idx = rng.gen_range(0..set.chars().count());
    set.chars().nth(idx).unwrap()
}

pub fn entropy_bits(password: &str) -> f64 {
    let mut pool = 0u32;
    if password.chars().any(|c| c.is_ascii_lowercase()) {
        pool += 26;
    }
    if password.chars().any(|c| c.is_ascii_uppercase()) {
        pool += 26;
    }
    if password.chars().any(|c| c.is_ascii_digit()) {
        pool += 10;
    }
    if password.chars().any(|c| SPECIAL.contains(c)) {
        pool += SPECIAL.len() as u32;
    }
    if pool == 0 {
        return 0.0;
    }
    password.chars().count() as f64 * (pool as f64).log2()
}

pub fn strength(password: &str) -> (f64, &'static str, crate::console::Color) {
    let bits = entropy_bits(password);
    if bits < 30.0 {
        (bits, "Very Weak", crate::console::Color::Fail)
    } else if bits < 50.0 {
        (bits, "Weak", crate::console::Color::Warning)
    } else if bits < 70.0 {
        (bits, "Medium", crate::console::Color::Cyan)
    } else if bits < 90.0 {
        (bits, "Strong", crate::console::Color::Green)
    } else {
        (bits, "Very Strong", crate::console::Color::Header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_password_meets_policy() {
        let pw = generate_password(20, 2, 3);
        assert_eq!(pw.chars().count(), 20);
        assert!(pw.chars().any(|c| c.is_ascii_lowercase()));
        assert!(pw.chars().any(|c| c.is_ascii_uppercase()));
        assert!(pw.chars().filter(|c| c.is_ascii_digit()).count() >= 3);
        assert!(pw.chars().filter(|c| SPECIAL.contains(*c)).count() >= 2);
    }

    #[test]
    fn short_length_is_clamped() {
        assert!(generate_password(3, 1, 1).chars().count() >= 8);
    }

    #[test]
    fn strength_labels() {
        assert_eq!(strength("aaaaaaaa").1, "Weak");
        assert_eq!(strength("A1!aB2@bC3#cD4$dE5%").1, "Very Strong");
    }
}
