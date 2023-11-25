use std::{fs, io};
use std::io::ErrorKind::NotFound;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use is_executable::is_executable;
use tempfile::TempDir;
use wait_timeout::ChildExt;
use crate::compiler::CompilationResult::*;
use crate::pipes::BufferedPipe;

pub(crate) enum CompilationResult {
    Success(PathBuf, Option<Duration>),
    InvalidExecutable(io::Error),
    CompilationError(String),
}

pub(crate) struct Compiler<'a> {
    pub(crate) tempdir: &'a TempDir,
    pub(crate) compile_timeout: Duration,
    pub(crate) compile_command: &'a str,
}

impl<'a> Compiler<'a> {
    fn is_source_file(path: &Path) -> bool {
        if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
            return matches!(extension, "cpp" | "cc" | "cxx" | "c");
        }
        !is_executable(path)
    }

    fn compile_cpp(&self, source_path: &Path, executable_path: &Path) -> Result<Duration, String> {
        let cmd = self.compile_command
            .replace("<IN>", source_path.to_str().expect("The provided filename is invalid!"))
            .replace("<OUT>", &executable_path.to_str().expect("The provided filename is invalid"));
        let mut split_cmd = cmd.split(" ");

        let mut stderr = BufferedPipe::create().expect("Failed to create stderr pipe");
        let time_before_compilation = Instant::now();
        let child = Command::new(&split_cmd.next().expect("The compile command is invalid!"))
            .args(split_cmd)
            .stderr(stderr.get_stdio())
            .spawn();

        let mut child = match child {
            Ok(child) => child,
            Err(error) if error.kind() == NotFound => { return Err("The compiler was not found!".to_string()) }
            Err(error) => { return Err(error.to_string()) }
        };

        match child.wait_timeout(self.compile_timeout).unwrap() {
            Some(status) => {
                if status.code().expect("The compiler returned an invalid status code") != 0 {
                    let compilation_result = stderr.join().expect("Failed to read compiler output");
                    return Err(compilation_result);
                }
            }
            None => {
                child.kill().unwrap();
                return Err("Compilation timed out".to_string());
            }
        }
        Ok(time_before_compilation.elapsed())
    }

    fn try_spawning_executable(executable_path: &PathBuf) -> io::Result<()> {
        Command::new(&executable_path)
            .spawn()
            .map(|mut child| {
                child.kill().expect("Failed to kill executable");
            })
    }

    pub(crate) fn prepare_executable(&self, source_path: &Path) -> CompilationResult {
        let source_stem = source_path.file_stem().expect("The provided filename is invalid!");
        let output_path = self.tempdir.path().join(format!("{}.o", source_stem.to_string_lossy()));

        if !Self::is_source_file(source_path) {
            fs::copy(source_path, &output_path).expect("The provided filename is invalid");
            if let Err(error) = Self::try_spawning_executable(&output_path) {
                return InvalidExecutable(error);
            }
            return Success(output_path, None);
        }

        match self.compile_cpp(source_path, &output_path) {
            Ok(compilation_time) => Success(output_path, Some(compilation_time)),
            Err(error) => CompilationError(error),
        }
    }
}