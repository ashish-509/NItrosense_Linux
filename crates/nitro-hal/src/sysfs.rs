use std::fs;
use std::path::Path;

pub(crate) fn read_trim<P: AsRef<Path>>(path: P) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

pub(crate) fn read_u64<P: AsRef<Path>>(path: P) -> Option<u64> {
    read_trim(path)?.parse().ok()
}

pub(crate) fn read_i64<P: AsRef<Path>>(path: P) -> Option<i64> {
    read_trim(path)?.parse().ok()
}
