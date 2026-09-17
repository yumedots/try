use crate::host::{host_cores, host_memory_gb};
use crate::paths::state_dir;

#[derive(Clone)]
pub(crate) struct Settings {
    pub(crate) cores: u32,
    pub(crate) mem: String,
    pub(crate) audio: bool,
}

pub(crate) const MEMORY: [&str; 7] = ["2G", "4G", "6G", "8G", "12G", "16G", "24G"];

pub(crate) fn memory_index(mem: &str) -> usize {
    MEMORY.iter().position(|value| *value == mem).unwrap_or(3)
}

pub(crate) fn read_settings() -> Settings {
    let mut settings = Settings {
        cores: host_cores().min(8),
        mem: if host_memory_gb() >= 16 {
            "8G".into()
        } else {
            "4G".into()
        },
        audio: cfg!(target_os = "macos"),
    };
    if let Ok(text) = std::fs::read_to_string(state_dir().join("settings")) {
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "cores" => settings.cores = value.parse().unwrap_or(settings.cores),
                "mem" => settings.mem = value.to_string(),
                "audio" => settings.audio = value == "on",
                _ => {}
            }
        }
    }
    settings
}

pub(crate) fn write_settings(settings: &Settings) {
    let dir = state_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let text = format!(
        "cores={}\nmem={}\naudio={}\n",
        settings.cores,
        settings.mem,
        if settings.audio { "on" } else { "off" }
    );
    let _ = std::fs::write(dir.join("settings"), text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn settings_file_matches_the_xmake_reader() {
        let dir = env::temp_dir().join("try-ui-settings-test");
        let _ = std::fs::remove_dir_all(&dir);
        env::set_var("TRY_STATE_DIR", &dir);
        let settings = Settings {
            cores: 6,
            mem: "12G".into(),
            audio: false,
        };
        write_settings(&settings);
        assert_eq!(
            std::fs::read_to_string(dir.join("settings")).unwrap(),
            "cores=6\nmem=12G\naudio=off\n"
        );
        let read = read_settings();
        assert_eq!(read.cores, 6);
        assert_eq!(read.mem, "12G");
        assert!(!read.audio);
        assert_eq!(MEMORY[memory_index(&read.mem)], "12G");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
