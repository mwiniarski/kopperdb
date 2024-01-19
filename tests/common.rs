use rand::{Rng, distributions::Alphanumeric};

pub const DB_PATH: &str = "testfiles";
pub const SEGMENT_SIZE: usize = 100;

pub fn random_key_value_with_size(size: usize) -> (String, String) {

    let get_random_str = || {
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(size)
            .map(char::from)
            .collect()
    };
    (get_random_str(), get_random_str())
}

pub fn random_key_value() -> (String, String) {
    const LEN: usize = 10;
    random_key_value_with_size(LEN)
}