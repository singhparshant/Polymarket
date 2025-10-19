use crate::modules::types::AppState;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

const STATE_PATH: &str = "state.json";

pub fn load_state() -> Option<AppState> {
    let path = Path::new(STATE_PATH);
    if !path.exists() {
        return None;
    }
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return None,
    };
    let mut buf = String::new();
    if file.read_to_string(&mut buf).is_err() {
        return None;
    }
    serde_json::from_str::<AppState>(&buf).ok()
}

pub fn save_state(state: &AppState) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(state).unwrap_or_else(|_| "{}".to_string());
    // let mut file = fs::File::create(STATE_PATH)?;
    // file.write_all(json.as_bytes())
    Ok(())
}
