mod common;
use crate::common::*;

use kopperdb::brass::*;

fn get_new_path() -> String {
    DB_PATH.to_owned() + "/kopper/" + &random_key_value_with_size(20).0
}

#[test]
fn test_write_read() {
    let brass = Brass::create(&get_new_path(), SEGMENT_SIZE).unwrap();

    // Write
    let (key, value) = random_key_value();
    brass.write(&key, &value).unwrap();

    // Read
    let read_response = brass.read(&key).unwrap();

    assert_eq!(read_response, value);
}

