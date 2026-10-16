#[derive(Debug, Clone)]
pub struct Config {
    pub addons_dir: String,
}

impl Default for Config {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Config {
            addons_dir: format!("{home}/.config/xedit/addons"),
        }
    }
}
