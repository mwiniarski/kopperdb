use std::time::Instant;

use rocket::State;
use rocket::serde::json::Json;
use rocket::fs::NamedFile;
use serde::Serialize;

use crate::kopper::Kopper;
use crate::stats::{Stats, self};

#[derive(Serialize)]
pub struct ReadResponse {
    value: String,
    error: String
}

#[derive(Serialize)]
pub struct WriteResponse {
    error: String
}

// api
#[get("/read/<key>")]
pub fn read(key: String, db: &State<Kopper>, stats: &State<Stats>) -> Json<ReadResponse> {
    let mut response = ReadResponse { 
        value: String::from(""), 
        error: String::from("OK") 
    };
    
    let timer = Instant::now();
    match db.read(key) {
        Ok(value_option) => {
            match value_option {
                Some(value) => {
                    response.value = value
                },
                None => {
                    response.error = "No such thing in database".to_string()
                }
            }
        },
        Err(err) => {
            response.error = err.to_string()
        }
    };
    
    stats.read_counter.lock().unwrap().push(timer.elapsed().as_nanos() as usize);

    Json(response)
}

#[get("/write/<key>/<value>")]
pub fn write(key: String, value: String, db: &State<Kopper>, stats: &State<Stats>) -> Json<WriteResponse> {
    let timer = Instant::now();

    let result = match db.write(key, value) {
        Ok(size) => {
            stats.size.lock().unwrap().push(size);
            "OK".to_string()
        },
        Err(err) => format!("Error writing to database! ({})", err)
    };

    stats.write_counter.lock().unwrap().push(timer.elapsed().as_nanos() as usize);
    Json(WriteResponse { error: result.to_string() })
}

pub fn create_kopper() -> Result<Kopper, std::io::Error> {
    Kopper::start("kopper.db")
}

pub fn create_stats() -> Stats {
    Stats::new()
}

#[get("/stats/<read_or_write>")]
pub async fn get_stats(read_or_write: String, stats: &State<Stats>) -> Option<NamedFile> {
    
    if read_or_write.eq("read") {
        {
            let read_counter = stats.read_counter.lock().unwrap();
            stats::draw(&*read_counter, "Reads", "us").expect("Drawing");
        }
        return NamedFile::open(std::path::Path::new("stats.png")).await.ok();
    }
    else if read_or_write.eq("write") {
        {
            let write_counter = stats.write_counter.lock().unwrap();
            stats::draw(&*write_counter, "Writes", "us").expect("Drawing");
        }
        return NamedFile::open(std::path::Path::new("stats.png")).await.ok()
    }
    else if read_or_write.eq("size") {
        {
            let size_metric = stats.size.lock().unwrap();
            stats::draw(&*size_metric, "Size", "KB").expect("Drawing");
        }
        return NamedFile::open(std::path::Path::new("stats.png")).await.ok()
    }

    None
}