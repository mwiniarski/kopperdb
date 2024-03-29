use std::{
    collections::{HashMap, BTreeMap}, 
    sync::{Mutex, mpsc::channel}, 
    sync::{Arc, mpsc::{Sender, Receiver}}, 
    fs::{File, OpenOptions, self}, 
    io::{Write, Read, self, Seek, SeekFrom},
    fmt::Display, 
    str::FromStr, 
    ops::Add
};

use crate::from_error;

#[derive(Clone)]
pub struct Kopper {
    state: Arc<Mutex<SharedState>>,
    compactor: Sender<()>,
    segment_size: usize,
    path: String
}

struct SharedState {
    table: HashMap<String, TableEntry>,
    files: BTreeMap<FileIndex, FileEntry>,
    offset: usize,
    current_file_index: FileIndex,
    size: usize
}

struct TableEntry {
    file_index: FileIndex,
    offset: usize,
    len: usize
}

#[derive(PartialEq, Eq, Ord, PartialOrd, Clone, Copy)]
struct FileIndex {
    base: u32,
    index: u32
}

struct FileEntry {
    file: File,
    unused_count: usize
}

impl Add<u32> for FileIndex {
    type Output = Self;

    fn add(mut self, rhs: u32) -> Self::Output {
        self.index += rhs;
        self
    }
}

impl Display for FileIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}_{}", self.base, self.index)
    }
}

impl FromStr for FileIndex {
    type Err = KopperError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (base, index) = s.split_once('_').ok_or(KopperError::InternalError(anyhow::anyhow!("Can't parse database file: {s}")))?;
        Ok(FileIndex { base: base.parse()?, index: index.parse()? })
    }
}

impl Kopper {
    pub fn create(path: &str, segment_size: usize) -> Result<Self, KopperError> {

        // Recover
        let shared_state = SharedState::create(path)?;

        // Use channel to communicate with compactor to make sure every compaction request is handled
        let (compactor_tx, compactor_rx) = channel::<()>();

        let ret = Kopper { 
            state: Arc::new(Mutex::new(shared_state)),
            compactor: compactor_tx,
            segment_size,
            path: path.to_owned(),
        };

        // Start background thread compacting segments to reclaim memory
        ret.run_compactor(compactor_rx);
        Ok(ret)
    }

    #[allow(dead_code)]
    pub fn size(&self) -> usize {
        self.state.lock().unwrap().size
    }

    #[allow(dead_code)]
    pub fn path(&self) -> String {
        self.path.clone()
    }

    pub fn read(&self, key: &str) -> Result<String, KopperError> {
        let state = self.state.lock().unwrap();

        let table_entry = match state.table.get(key) {
            Some(table_entry) => table_entry,
            None => return Err(KopperError::KeyDoesNotExist(key.to_owned())),
        };

        let mut file = 
        state.files
            .get(&table_entry.file_index).unwrap() // Can't recover from this. Should panic.
            .file.try_clone().unwrap();

        // TODO: This is OK because files are never deleted. 

        let offset = table_entry.offset;
        let mut buffer = vec![0; table_entry.len];

        file.seek(SeekFrom::Start(offset as u64))?;
        file.read_exact(&mut buffer)?;

        Ok(String::from_utf8(buffer)?)
    }

    pub fn write(&self, key: &str, value: &str) -> Result<usize, KopperError> {
        
        let mut state = self.state.lock().unwrap();

        let key_len = key.as_bytes().len();
        let value_len = value.as_bytes().len();

        // 0. Segment file if next entry would exceed max size
        if key_len + value_len + 2 + state.offset > self.segment_size {
            self.cut_off_segment(&mut state);

            // Ok to unwrap because sender always exists until receiver exists
            self.compactor.send(()).unwrap(); 
        }

        // 1. Save in in-memory map
        let entry = TableEntry {
            file_index: state.current_file_index,
            offset: state.offset + key.as_bytes().len() + 1,
            len: value.as_bytes().len()
        };

        let result = state.table.insert(key.to_string(), entry);
        match result {
            Some(entry) => {
                println!("{}", &entry.file_index);
                state.files.get_mut(&entry.file_index).unwrap().unused_count += 1;
            }
            None => {},
        }

        // 2. Write to disk
        let mut string_to_save = key.to_string();
        string_to_save.push('\0');
        string_to_save.push_str(&value);
        string_to_save.push('\0');
        
        let string_to_save = string_to_save.as_bytes();
        let file_index = state.current_file_index.clone();
        state.files.get_mut(&file_index).unwrap().file.write_all(string_to_save)?;

        // Update current offset and total size
        state.offset += string_to_save.len();
        state.size += string_to_save.len();

        Ok(state.size)
    }

    fn cut_off_segment(&self, state: &mut std::sync::MutexGuard<'_, SharedState>) {
              
        // Increment index - current_file_index is the biggest of all
        state.current_file_index = FileIndex { base: state.current_file_index.base + 1, index: 0 };
        let new_file_name = self.path.clone() + "/" + &state.current_file_index.to_string();

        // Create a new file
        let file = OpenOptions::new()
                        .read(true)
                        .append(true)
                        .create(true)
                        .open(new_file_name)
                        .expect("Failed to open file");

        // Add new file to file table
        let new_file_index = state.current_file_index;
        state.files.insert(new_file_index, FileEntry { file: file, unused_count: 0 });
        state.offset = 0;        
    }

    fn run_compactor(&self, receiver: Receiver<()>) {

        let state = self.state.clone();
        let path = self.path.clone();
        std::thread::spawn(move || {

            fn compact(state_mutex: &Mutex<SharedState>, path: String) {

                // Release the lock immidiately after taking a copy of current state
                let state = state_mutex.lock().unwrap();

                // Choose the best file to compact
                let (mut file_index, mut file_entry) = state.files.first_key_value().unwrap();
                for (index, entry) in state.files.iter() {
                    if entry.unused_count > file_entry.unused_count {
                        file_index = index;
                        file_entry = entry;
                    }
                }
                
                // Make explicit copies
                let file_index = *file_index;
                let mut file: File = file_entry.file.try_clone().unwrap();
                drop(state);
                
                // Load file into memory
                let mut buffer = Vec::new();
                file.seek(io::SeekFrom::Start(0)).unwrap();
                file.read_to_end(&mut buffer).unwrap();
                
                let mut new_file_contents = Vec::new();
                let iter = KeyValueIterator::from(&buffer);
                let compacted_file_index = file_index + 1;

                // Locked hashmap access here
                let mut lock = state_mutex.lock().unwrap();
                for (key, key_value, value_offset) in iter {
                    
                    // If the newest entry exists in the file that's being compacted, 
                    // change it's file_index and offset to new file
                    let entry = lock.table.get(key).unwrap();
                    if entry.file_index == file_index && entry.offset == value_offset {
                        lock.table.insert(key.to_owned(), TableEntry { 
                            file_index: compacted_file_index, 
                            offset: new_file_contents.len() + key.len() + 1, 
                            len: key_value.len() - key.len() - 2
                        });
                        new_file_contents.extend_from_slice(key_value);
                    }
                }

                // Save compacted file
                if !new_file_contents.is_empty() {
                    let mut compacted_file =
                        OpenOptions::new()
                            .append(true)
                            .create(true)
                            .open(path.clone() + "/" + &compacted_file_index.to_string())
                            .expect("Can't open file in compactor");
                    
                    compacted_file.write_all(&new_file_contents).unwrap();
                    
                    // When all is ready, insert the new file to master tree
                    lock.files.insert(compacted_file_index, FileEntry { file: compacted_file, unused_count: 0 });
                    lock.size += new_file_contents.len();
                }

                lock.size -= file.metadata().unwrap().len() as usize;
                lock.files.remove(&file_index);
                fs::remove_file(path + "/" + &file_index.to_string()).unwrap();
                println!("Removed {}", file_index);
            }

            loop {
                match receiver.recv() {
                    Ok(_) => compact(&*state, path.clone()),
                    Err(_) => { break; }, // All senders are dropped
                }
            }
            
            println!("{}", state.lock().unwrap().offset);
        });
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KopperError {
    #[error(transparent)]
    InternalError(anyhow::Error),

    #[error("No such item: {0}")]
    KeyDoesNotExist(String)
}

from_error!(KopperError::InternalError, std::num::ParseIntError, std::io::Error, std::str::Utf8Error, std::string::FromUtf8Error);

impl SharedState {
    fn create(path: &str) -> Result<SharedState, KopperError> {
        let mut state = SharedState {
            table: HashMap::new(),
            files: BTreeMap::new(),
            offset: 0,
            current_file_index: FileIndex { base: 0, index: 0 },
            size: 0,
        };

        // Create dir if doesn't exist yet
        match fs::create_dir_all(path) { _ => () };

        // Recover all files
        for dir_entry in fs::read_dir(path)? {

            let dir_entry = dir_entry?;

            let mut file = 
                OpenOptions::new()
                    .read(true)
                    .append(true)
                    .create(true)
                    .open(dir_entry.path())?;
            
            let file_index: FileIndex = 
                dir_entry.path()
                    .file_name().unwrap()
                    .to_str().unwrap()
                    .parse()?;

            println!("Recovering file: {}", file_index);

            state.size += SharedState::recover_file(&mut state.table, file_index, &mut file)?;
            state.files.insert(file_index, FileEntry { file, unused_count: 0 });
        }

        // If starting a new database, create the first file
        if state.files.is_empty() {
            let head_file = String::from(path) + "/" + &state.current_file_index.to_string();
            let file = OpenOptions::new()
                .read(true)
                .append(true)
                .create(true)
                .open(head_file)?;

            state.files.insert(FileIndex { base: 0, index: 0 }, FileEntry { file, unused_count: 0 });
        }

        // TODO: update unused counters for all files

        state.current_file_index = *state.files.first_key_value().unwrap().0;
        state.offset = state.files.first_key_value().unwrap().1.file.metadata().unwrap().len() as usize;
        Ok(state)
    }

    fn recover_file(table: &mut HashMap<String, TableEntry>, file_index: FileIndex, file: &mut File) -> Result<usize, KopperError> {

        enum CurrentlyReading { Key, Value }
        let mut currently_reading = CurrentlyReading::Key;
        let mut key = String::new();

        // With regards to current buffer
        let mut key_offset: usize;

        // With regards to file
        let mut value_file_offset: usize = 0; 
        let mut buffer_file_offset: usize = 0;
        
        let mut buffer = [0; 2048];

        loop {
            let bytes_in_buffer = match file.read(&mut buffer)? {
                0 => break,
                bytes_read => bytes_read,
            };
            
            key_offset = 0;
            
            for byte_index in 0..bytes_in_buffer {
                
                if buffer[byte_index] == b'\0' {

                    // TODO: if this is first byte: ERROR
                    
                    match currently_reading {
                        CurrentlyReading::Key => {
                            key.push_str(std::str::from_utf8(&buffer[key_offset..byte_index]).unwrap());
                            
                            value_file_offset = buffer_file_offset + byte_index + 1;
                            currently_reading = CurrentlyReading::Value;
                        },
                        CurrentlyReading::Value => {
                            // Swap pointers between key and empty string to avoid cloning
                            let mut tmp_key = String::new();
                            std::mem::swap(&mut tmp_key, &mut key);
                            
                            // Collected all needed parts: key, value's offset and length
                            table.insert(tmp_key, 
                                TableEntry {
                                    file_index,
                                    offset: value_file_offset,
                                    len: buffer_file_offset + byte_index - value_file_offset,
                                });
                                
                            key_offset = byte_index + 1;
                            currently_reading = CurrentlyReading::Key;
                        }
                    }
                }
            }

            // Being here, we're probably left with some incomplete key or value that continues in the next chunk
            match currently_reading {
                CurrentlyReading::Key => {
                    key.push_str(std::str::from_utf8(&buffer[key_offset..bytes_in_buffer])?);
                },
                _ => ()
            }

            buffer_file_offset += bytes_in_buffer;
        }

        Ok(buffer_file_offset)
    }
}

/// [`KeyValueIterator`] is an iterator that given a &Vec<u8> of format 
/// `['k','e','y','\0','v','a','l','u','e','\0']` iterates over key-value pairs.
/// 
/// Iterator returns a tuple containing `key` string, ref to slice with `value`, and `offset`
/// related to the beginning of the vector.  
/// 
/// ```
/// use kopperdb::kopper::KeyValueIterator;
///
/// let buffer: &[u8] = b"AB\0CD\0EF\0GH\0";
/// 
/// for (key, value, offset) in KeyValueIterator::from(buffer)  {
///     println!("{}: {}, at {}", key, std::str::from_utf8(value).unwrap(), offset);
/// }
/// ```
pub struct KeyValueIterator<'a> {
    buf: &'a [u8],
    pointer: usize
}

impl<'a> KeyValueIterator<'a> {
    pub fn from(buf: &'a [u8]) -> Self {
        KeyValueIterator { buf, pointer: 0 }
    }
}

impl<'a> Iterator for KeyValueIterator<'a> {
    type Item = (&'a str,&'a [u8],usize);

    fn next(&mut self) -> Option<Self::Item> {
        let mut key = "";
        let mut value: &[u8] = &[];
        let mut key_found = false;
        let mut offset = 0;

        // Find key
        for byte in self.pointer..self.buf.len() {
            if self.buf[byte] == b'\0' {
                if !key_found {
                    key = std::str::from_utf8(&self.buf[self.pointer..byte]).unwrap();
                    key_found = true;
                    offset = byte + 1;
                }
                else {
                    value = &self.buf[self.pointer..byte + 1];
                    self.pointer = byte + 1;
                    break;
                }
            }
        }
        
        if key.is_empty() || value.is_empty() {
            return None;
        }

        Some((key, value, offset))
    }
}