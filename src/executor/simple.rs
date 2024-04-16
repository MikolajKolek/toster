use std::ffi::{c_char, c_ulong, c_void, CString};
use std::fs::File;
use std::io::Error;
use std::mem::{size_of, size_of_val, zeroed};
use std::os::fd::{AsFd, AsRawFd};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};
use crate::test_errors::{ExecutionError, ExecutionMetrics};
use wait_timeout::ChildExt;
use crate::executor::TestExecutor;
use crate::test_errors::ExecutionError::{RuntimeError, TimedOut};

use libc::{__u64, c_int, c_long, CLD_EXITED, close, dup2, execv, F_SETFD, fcntl, FD_CLOEXEC, fork, id_t, kill, MAP_ANONYMOUS, MAP_SHARED, memset, mmap, munmap, O_CLOEXEC, O_CREAT, O_RDONLY, O_WRONLY, open, P_PID, PROT_READ, PROT_WRITE, pthread_barrier_destroy, pthread_barrier_init, pthread_barrier_t, pthread_barrier_wait, pthread_barrierattr_destroy, pthread_barrierattr_init, pthread_barrierattr_setpshared, pthread_barrierattr_t, PTHREAD_PROCESS_SHARED, read, S_IRUSR, S_IWUSR, siginfo_t, SIGKILL, vfork, wait, waitid, WEXITED, WNOWAIT, WSTOPPED};
use perf_event_open_sys::bindings::{PERF_COUNT_HW_INSTRUCTIONS, perf_event_attr, PERF_FLAG_FD_CLOEXEC, PERF_FLAG_FD_NO_GROUP, PERF_TYPE_HARDWARE};
use perf_event_open_sys::perf_event_open;

#[cfg(all(unix))]
use crate::generic_utils::halt;
#[cfg(all(unix))]
use std::os::unix::process::ExitStatusExt;
use std::ptr::null;

pub(crate) struct SimpleExecutor {
    pub(crate) timeout: Duration,
    pub(crate) executable_path: PathBuf,
}

impl SimpleExecutor {
    fn map_status_code(status: &ExitStatus) -> Result<(), ExecutionError> {
        match status.code() {
            Some(0) => Ok(()),
            Some(exit_code) => {
                Err(RuntimeError(format!("- the program returned a non-zero return code: {}", exit_code)))
            },
            None => {
                #[cfg(all(unix))]
                if status.signal().expect("The program returned an invalid status code") == 2 {
                    halt();
                }

                Err(RuntimeError(format!("- the process was terminated with the following error:\n{}", status.to_string())))
            }
        }
    }

    fn wait_for_child(&self, mut child: Child) -> (ExecutionMetrics, Result<(), ExecutionError>) {
        let start_time = Instant::now();
        let status = child.wait_timeout(self.timeout).unwrap();

        match status {
            Some(status) => (
                ExecutionMetrics { time: Some(start_time.elapsed()), memory_kibibytes: None },
                SimpleExecutor::map_status_code(&status)
            ),
            None => {
                child.kill().unwrap();
                (ExecutionMetrics { time: Some(self.timeout), memory_kibibytes: None }, Err(TimedOut))
            }
        }
    }
}

impl TestExecutor for SimpleExecutor {
    fn test_to_stdio(&self, input_stdio: &File, output_stdio: &File) -> (ExecutionMetrics, Result<(), ExecutionError>) {
        /*let child = Command::new(&self.executable_path)
            .stdin(Stdio::from(input_stdio.try_clone().unwrap()))
            .stdout(Stdio::from(output_stdio.try_clone().unwrap()))
            .stderr(Stdio::null())
            .spawn().expect("Failed to spawn child");

        self.wait_for_child(child)*/

        let test: i32 = input_stdio.as_raw_fd();
        (
            ExecutionMetrics { time: Some(Duration::new(0, (run_sio2jail(&self.executable_path.to_str().unwrap(), input_stdio.as_raw_fd(), output_stdio.as_raw_fd()) / 2) as u32)), memory_kibibytes: None },
            Ok(())
        )
    }
}

fn run_sio2jail(executable_path: &str, input_path: i32, output_path: i32) -> u64 {
    unsafe {
        //onPreFork
        //let test: *mut c_void = null();
        let barrier_: *mut pthread_barrier_t = mmap(0 as *mut c_void, size_of::<pthread_barrier_t>(), PROT_READ | PROT_WRITE, MAP_ANONYMOUS | MAP_SHARED, 0, 0) as *mut pthread_barrier_t;
        let mut attr: pthread_barrierattr_t = zeroed();
        pthread_barrierattr_init(&mut attr);
        pthread_barrierattr_setpshared(&mut attr, PTHREAD_PROCESS_SHARED);
        pthread_barrier_init(barrier_, &mut attr, 2);
        pthread_barrierattr_destroy(&mut attr);
        
        let child_pid = fork();

        if child_pid < 0 {
            println!("{}", Error::last_os_error().raw_os_error().unwrap())
        }
        if child_pid == 0 {
            //onPostForkChild
            pthread_barrier_wait(barrier_);
            munmap(barrier_ as *mut c_void, size_of::<pthread_barrier_t>());

            let exec_c_str = CString::new(executable_path).unwrap();
            //let input_c_str = CString::new(input_path).unwrap();
            //let output_c_str = CString::new(output_path).unwrap();

            //let input = open(input_c_str.as_ptr(), O_RDONLY | O_CLOEXEC | O_CREAT, S_IRUSR | S_IWUSR);
            //let output = open(output_c_str.as_ptr(), O_WRONLY | O_CLOEXEC | O_CREAT, S_IRUSR | S_IWUSR);
            dup2(input_path as c_int, 0); dup2(output_path as c_int, 1);
            close(input_path); close(output_path);

            //let arr: *const *const c_char = libc::malloc(1) as *const *const c_char;
            let arg: [*const c_char; 2] = [exec_c_str.as_ptr(), null()];

            //let arr: *const *const c_char = libc::malloc(0) as *const *const c_char ;
            execv(exec_c_str.as_ptr(), arg.as_ptr());

            println!("fuck")
        }
        else {
            //onPostForkParent
            let mut attrs: perf_event_attr = zeroed();
            memset(&mut attrs as *mut perf_event_attr as *mut c_void, 0, size_of::<perf_event_attr>());
            //memset((&mut attrs) as *mut c_void, 0, size_of::<perf_event_attr>());
            attrs.type_ = PERF_TYPE_HARDWARE;
            attrs.size = size_of_val(&mut attrs) as u32;
            attrs.config = PERF_COUNT_HW_INSTRUCTIONS as __u64;
            attrs.set_exclude_user(0);
            attrs.set_exclude_kernel(1);
            attrs.set_exclude_hv(1);
            attrs.set_disabled(1);
            attrs.set_enable_on_exec(1);
            attrs.set_inherit(1);
            let perf_fd = perf_event_open(&mut attrs, child_pid, -1, -1, (PERF_FLAG_FD_NO_GROUP | PERF_FLAG_FD_CLOEXEC) as c_ulong);
            //println!("{} {}\n", perf_fd, Error::last_os_error().raw_os_error().unwrap());
            //fcntl(perf_fd, F_SETFD, FD_CLOEXEC);
            pthread_barrier_wait(barrier_);
            pthread_barrier_destroy(barrier_);
            munmap(barrier_ as *mut c_void, size_of::<pthread_barrier_t>());

            while true {
                let mut waitinfo: siginfo_t = zeroed();
                let ret_val = waitid(P_PID, child_pid as id_t, &mut waitinfo as *mut siginfo_t, WEXITED | WSTOPPED | WNOWAIT);

                if(waitinfo.si_code == CLD_EXITED) {
                    //let mut res: i64 = 1;
                    //for i in 1..10000000 {
                    //    res = res ^ i;
                    //}
                    //println!("{}, {}", ret_val, res);

                    let mut instructions_used: i64 = 0;
                    let size = read(perf_fd, &mut instructions_used as *mut c_long as *mut c_void, size_of_val(&instructions_used));
                    if (size != size_of_val(&instructions_used) as isize) {
                        println!("ERROR")
                    }
                    if (instructions_used < 0) {
                        println!("ERROR2")
                    }

                    close(perf_fd);
                    wait(child_pid as *mut c_int);
                    //kill(child_pid, SIGKILL);
                    //println!("{}", perf_fd);
                    //println!("{}", instructions_used);
                    return instructions_used as u64;
                }
            }
        }
    }
    
    return 0;
}

