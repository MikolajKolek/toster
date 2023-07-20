#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use {
	std::{fs, process},
	std::fs::Permissions,
	std::os::unix::fs::PermissionsExt,
	directories::BaseDirs
};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
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

	fs::create_dir_all(executable_dir_str).unwrap_or_else(|_| {
		println!("cargo:warning=Couldn't create the {} directory. Sio2jail was not installed", executable_dir_str);
		process::exit(0);
	});
	fs::copy("sio2jail", format!("{}/sio2jail", executable_dir_str)).unwrap_or_else(|_| {
		println!("cargo:warning=Couldn't copy sio2jail to {}. Sio2jail was not installed", executable_dir_str);
		process::exit(0);
	});
	fs::set_permissions(format!("{}/sio2jail", executable_dir_str), Permissions::from_mode(0o755)).unwrap_or_else(|_| {
		println!("cargo:warning=Couldn't set execute permissions on sio2jail at {}. Sio2jail was not installed", format!("{}/sio2jail", executable_dir_str));
		process::exit(0);
	});
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn main() {}