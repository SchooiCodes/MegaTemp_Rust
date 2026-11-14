use rand::distributions::Alphanumeric;
use rand::Rng;

pub fn random_alnum(n: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(n)
        .map(char::from)
        .collect()
}

pub const FIRST_NAMES: &[&str] = &[
    "James", "Mary", "Robert", "Patricia", "John", "Jennifer", "Michael", "Linda", "David",
    "Elizabeth", "William", "Barbara", "Richard", "Susan", "Joseph", "Jessica", "Thomas",
    "Sarah", "Charles", "Karen", "Christopher", "Nancy", "Daniel", "Lisa", "Matthew",
    "Betty", "Anthony", "Margaret", "Mark", "Sandra", "Donald", "Ashley",
];

pub fn random_first_name() -> &'static str {
    FIRST_NAMES[rand::thread_rng().gen_range(0..FIRST_NAMES.len())]
}

/// Expand a leading `~` the way Python's `os.path.expanduser` does.
pub fn expand_user(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    } else if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.to_string_lossy().to_string();
        }
    }
    path.to_string()
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn prompt(label: &str) -> String {
    use std::io::Write;
    print!("{label}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    line.trim_end_matches(['\r', '\n']).to_string()
}

pub fn prompt_yes_no(label: &str, default_yes: bool) -> bool {
    let suffix = if default_yes { " [Y/n]: " } else { " [y/N]: " };
    loop {
        let answer = prompt(&format!("{label}{suffix}")).to_lowercase();
        if answer.is_empty() {
            return default_yes;
        }
        match answer.as_str() {
            "y" | "yes" => return true,
            "n" | "no" => return false,
            _ => println!("Please answer yes or no."),
        }
    }
}
