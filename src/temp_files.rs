use std::process::Stdio;
use memfile::MemFile;

pub(crate) fn make_cloned_stdio(mem_file: &MemFile) -> Stdio {
    Stdio::from(mem_file.try_clone().unwrap())
}
