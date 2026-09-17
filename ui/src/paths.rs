use std::{env, path::{Path, PathBuf}};

pub(crate) fn state_dir() -> PathBuf {
    env::var_os("TRY_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| project_dir().join("state"))
}

pub(crate) fn project_dir() -> PathBuf {
    PathBuf::from(env::var("TRY_PROJECT_DIR").unwrap_or_else(|_| ".".into()))
}

pub(crate) fn serial_log(project: &Path) -> PathBuf {
    env::var_os("TRY_SERIAL_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|| project.join("build/guest.log"))
}
