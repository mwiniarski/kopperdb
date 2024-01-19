mod common;
use core::time;

use kopperdb::kopper::Kopper;

use crate::common::*;

fn get_new_path() -> String {
    DB_PATH.to_owned() + "/kopper/" + &random_key_value_with_size(20).0
}

#[test]
fn test_write_read() {

    let kopper = Kopper::create(&get_new_path(), SEGMENT_SIZE).unwrap();

    // Write
    let (key, value) = random_key_value();
    kopper.write(&key, &value).unwrap();

    // Read
    let read_response = kopper.read(&key).unwrap();

    assert_eq!(read_response, value);
}


#[test]
fn database_recovers_after_dying() {

    let kopper = Kopper::create(&get_new_path(), SEGMENT_SIZE).unwrap();

    let mut key_values = Vec::new();
    for i in 0..5 {
        key_values.push(random_key_value());
        kopper.write(&key_values[i].0, &key_values[i].1).unwrap();
    }

    // All in-memory structure is dropped
    let kopper = Kopper::create(&kopper.path(), SEGMENT_SIZE).unwrap();
    
    for i in key_values {
        let read_response = kopper.read(&i.0).unwrap();
        assert_eq!(read_response, i.1);
    }
}

#[test]
fn recover_all_files_from_folder() {
    // Create small segments
    let kopper = Kopper::create(&get_new_path(), SEGMENT_SIZE).unwrap();
    
    // Fill first file quickly
    for _ in 0..3 {
        let (key, value) = random_key_value_with_size(19);
        kopper.write(&key, &value).unwrap();
    }

    // Meaningful value - should be in second file
    kopper.write("meaningful", "thing").unwrap();

    let read_response = kopper.read("meaningful").unwrap();
    assert_eq!(&read_response, "thing");
}

#[test]
fn database_does_not_grow_forever() {
    let kopper = Kopper::create(&get_new_path(), 14).unwrap();

    // Send 10 identical requests
    let (key, value) = random_key_value_with_size(2);
    for _ in 0..10 {
        kopper.write(&key, &value).unwrap();
        std::thread::sleep(time::Duration::from_millis(10));
    }

    // Verify that database is smaller than 10 x (key + value + 2)
    let all_entries_together_size = 10 * (2 + 2 + 2) / 2;
    let size = kopper.size();
    
    assert!(size < all_entries_together_size, "{} >= {}", size, all_entries_together_size);
}

#[test]
fn file_offset_is_set_correctly_after_recovery() {
    let kopper = Kopper::create(&get_new_path(), SEGMENT_SIZE).unwrap();

    // Write to a file - offset is len(key + value) + 2
    kopper.write("some_key", "222222").unwrap();

    // Recreate memory part of database from files
    let kopper = Kopper::create(&kopper.path(), SEGMENT_SIZE).unwrap();
    
    // Write to a file again - offset should be recovered too, and correctly saved in in-memory table
    kopper.write("some_key", "333333").unwrap();

    let read_response = kopper.read("some_key").unwrap();
    assert_eq!(read_response, "333333");
}