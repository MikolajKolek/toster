use std::{fs, process};
use directories::{BaseDirs};

#[cfg(target_os = "linux")]
#[cfg(target_arch = "x86_64")]
fn main() {
	let base_dirs = BaseDirs::new().unwrap_or_else(|| {
		println!("cargo:warning=No valid home directory path could be retrieved from the operating system. Sio2jail was not installed");
		process::exit(0)
	});
	let executable_dir = base_dirs.executable_dir().unwrap_or_else(|| {
		println!("cargo:warning=Couldn't locate the user's executable directory. Sio2jail was not installed");
		process::exit(0);
	});
	let executable_dir_str = executable_dir.to_str().unwrap_or_else(|| {
		println!("cargo:warning=The user's executable directory is invalid. Sio2jail was not installed");
		process::exit(0);
	});

	fs::copy("sio2jail", format!("{}/sio2jail", executable_dir_str)).unwrap_or_else(|_| {
		println!("cargo:warning=Couldn't copy sio2jail to {}. Sio2jail was not installed", executable_dir_str);
		process::exit(0);
	});
}