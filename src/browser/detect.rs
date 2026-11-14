use std::path::PathBuf;

pub fn detect_browser() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    #[cfg(target_os = "windows")]
    {
        let program_files = std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Program Files"));
        let program_files_x86 = std::env::var_os("ProgramFiles(x86)")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Program Files (x86)"));
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_default();
        candidates.extend([
            program_files.join("Google\\Chrome\\Application\\chrome.exe"),
            program_files_x86.join("Google\\Chrome\\Application\\chrome.exe"),
            local.join("Google\\Chrome\\Application\\chrome.exe"),
            program_files.join("BraveSoftware\\Brave-Browser\\Application\\brave.exe"),
            program_files_x86.join("BraveSoftware\\Brave-Browser\\Application\\brave.exe"),
            local.join("BraveSoftware\\Brave-Browser\\Application\\brave.exe"),
            program_files.join("Microsoft\\Edge\\Application\\msedge.exe"),
            program_files_x86.join("Microsoft\\Edge\\Application\\msedge.exe"),
            local.join("Chromium\\Application\\chrome.exe"),
        ]);
    }
    #[cfg(target_os = "linux")]
    {
        for name in [
            "chromium",
            "chromium-browser",
            "google-chrome",
            "google-chrome-stable",
            "brave-browser",
            "microsoft-edge",
        ] {
            candidates.push(PathBuf::from(format!("/usr/bin/{name}")));
            candidates.push(PathBuf::from(format!("/usr/local/bin/{name}")));
        }
    }
    #[cfg(target_os = "macos")]
    {
        candidates.extend([
            PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
            PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"),
            PathBuf::from("/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"),
            PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
        ]);
    }
    candidates.into_iter().find(|p| p.is_file())
}
