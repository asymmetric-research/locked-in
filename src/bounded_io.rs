use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

pub const MAX_SOURCE_FILE_SIZE: usize = 16 * 1024 * 1024;
pub const MAX_CONFIG_FILE_SIZE: usize = 2 * 1024 * 1024;
pub const MAX_GIT_INDEX_SIZE: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub enum BoundedReadError {
    Io(io::Error),
    InvalidUtf8,
    Allocation,
    Symlink,
    TooLarge { max: usize },
}

impl fmt::Display for BoundedReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "could not be read: {error}"),
            Self::InvalidUtf8 => formatter.write_str("is not valid UTF-8"),
            Self::Allocation => formatter.write_str("could not be buffered within memory limits"),
            Self::Symlink => formatter.write_str("is a symbolic link"),
            Self::TooLarge { max } => write!(formatter, "exceeds the {max}-byte size limit"),
        }
    }
}

pub fn read_bounded_bytes(
    path: &Path,
    max_size: usize,
    reject_symlinks: bool,
) -> Result<Vec<u8>, BoundedReadError> {
    let metadata = fs::symlink_metadata(path).map_err(BoundedReadError::Io)?;
    if reject_symlinks && metadata.file_type().is_symlink() {
        return Err(BoundedReadError::Symlink);
    }
    if metadata.len() > u64::try_from(max_size).unwrap_or(u64::MAX) {
        return Err(BoundedReadError::TooLarge { max: max_size });
    }

    let mut file = File::open(path).map_err(BoundedReadError::Io)?;
    let initial_capacity = metadata.len().try_into().unwrap_or(max_size).min(max_size);
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(initial_capacity)
        .map_err(|_| BoundedReadError::Allocation)?;
    let mut chunk = [0u8; 8 * 1024];

    loop {
        let bytes_read = file.read(&mut chunk).map_err(BoundedReadError::Io)?;
        if bytes_read == 0 {
            break;
        }
        let new_length = bytes
            .len()
            .checked_add(bytes_read)
            .ok_or(BoundedReadError::TooLarge { max: max_size })?;
        if new_length > max_size {
            return Err(BoundedReadError::TooLarge { max: max_size });
        }
        bytes
            .try_reserve_exact(bytes_read)
            .map_err(|_| BoundedReadError::Allocation)?;
        bytes.extend_from_slice(&chunk[..bytes_read]);
    }
    Ok(bytes)
}

pub fn read_bounded_utf8(
    path: &Path,
    max_size: usize,
    reject_symlinks: bool,
) -> Result<String, BoundedReadError> {
    let bytes = read_bounded_bytes(path, max_size, reject_symlinks)?;
    String::from_utf8(bytes).map_err(|_| BoundedReadError::InvalidUtf8)
}
