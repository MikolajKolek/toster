use std::fs::File;
use std::io;
use std::process::Stdio;
#[cfg(target_os = "linux")]
use memfile::MemFile;

pub(crate) fn make_cloned_stdio(mem_file: &File) -> Stdio {
    Stdio::from(mem_file.try_clone().unwrap())
}

#[cfg(target_os = "linux")]
pub(crate) fn create_temp_file() -> io::Result<File> {
    // The file is deleted when all file descriptors are closed
    // https://man7.org/linux/man-pages/man2/memfd_create.2.html
    Ok(MemFile::create_default("toster temporary file")?.into_file())
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn create_temp_file() -> io::Result<File> {
    // tempfile() adds FILE_FLAG_DELETE_ON_CLOSE flag on Windows and TMPFILE on Linux
    // so the file should be deleted when all file descriptors are closed
    tempfile::tempfile()
}