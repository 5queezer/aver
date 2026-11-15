use std::path::Path;

pub struct Store;

impl Store {
    pub fn open(path: &Path) -> Store {
        validate(path);
        Store
    }
}

fn validate(_path: &Path) {}
