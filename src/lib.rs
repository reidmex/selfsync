mod mapping;
mod proxy;

use std::env;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::sync::OnceLock;

use mapping::AccountMapping;

const UPSTREAM_URL: &str = "https://clients4.google.com/chrome-sync";

static MAPPING: OnceLock<AccountMapping> = OnceLock::new();

type MainFn = unsafe extern "C" fn(c_int, *mut *mut c_char, *mut *mut c_char) -> c_int;

static mut REAL_MAIN: Option<MainFn> = None;

unsafe extern "C" fn wrapped_main(
    argc: c_int,
    argv: *mut *mut c_char,
    envp: *mut *mut c_char,
) -> c_int {
    unsafe {
        if let Some(main) = REAL_MAIN {
            main(argc, argv, envp)
        } else {
            1
        }
    }
}

/// 判断是否是 Chrome browser 主进程：
/// 1. argv[0] 必须以 "chrome" 结尾（排除 grep、readlink 等系统命令）
/// 2. 没有 --type= 参数（排除 renderer、gpu 等子进程）
fn is_chrome_browser_process(argc: c_int, argv: *mut *mut c_char) -> bool {
    if argc < 1 {
        return false;
    }

    // 检查 argv[0] 是否是 chrome 二进制
    let argv0 = unsafe { CStr::from_ptr(*argv) };
    let is_chrome = argv0
        .to_str()
        .is_ok_and(|s| s.ends_with("/chrome") || s.ends_with("/chrome-stable") || s == "chrome");

    if !is_chrome {
        return false;
    }

    // 排除带 --type= 的子进程
    for i in 1..argc as isize {
        let arg = unsafe { CStr::from_ptr(*argv.offset(i)) };
        if let Ok(s) = arg.to_str()
            && s.starts_with("--type=")
        {
            return false;
        }
    }
    true
}

fn get_switch_value(argc: c_int, argv: *mut *mut c_char, name: &str) -> Option<String> {
    let prefix = format!("--{name}=");
    for i in 0..argc as isize {
        let arg = unsafe { CStr::from_ptr(*argv.offset(i)) };
        if let Ok(s) = arg.to_str()
            && let Some(value) = s.strip_prefix(&prefix)
        {
            return Some(value.to_string());
        }
    }
    None
}

fn default_user_data_dir() -> String {
    let home = env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    format!("{home}/.config/google-chrome")
}

pub fn get_mapping() -> Option<&'static AccountMapping> {
    MAPPING.get()
}

/// # Safety
/// Called by the dynamic linker as the process entry point.
/// `argv` must point to a valid null-terminated array of `argc` C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __libc_start_main(
    main: MainFn,
    argc: c_int,
    argv: *mut *mut c_char,
    init: Option<unsafe extern "C" fn()>,
    fini: Option<unsafe extern "C" fn()>,
    rtld_fini: Option<unsafe extern "C" fn()>,
    stack_end: *mut c_void,
) -> c_int {
    let real_start_main: unsafe extern "C" fn(
        MainFn,
        c_int,
        *mut *mut c_char,
        Option<unsafe extern "C" fn()>,
        Option<unsafe extern "C" fn()>,
        Option<unsafe extern "C" fn()>,
        *mut c_void,
    ) -> c_int = unsafe {
        let sym = libc::dlsym(libc::RTLD_NEXT, c"__libc_start_main".as_ptr());
        std::mem::transmute(sym)
    };

    // 子进程直接放行
    if !is_chrome_browser_process(argc, argv) {
        unsafe {
            REAL_MAIN = Some(main);
            return real_start_main(wrapped_main, argc, argv, init, fini, rtld_fini, stack_end);
        }
    }

    let user_data_dir =
        get_switch_value(argc, argv, "user-data-dir").unwrap_or_else(default_user_data_dir);

    eprintln!("[lzc-sync] browser process detected, user-data-dir: {user_data_dir}");

    // 构建 cache_guid → email 映射表
    let account_mapping = AccountMapping::build(&user_data_dir);
    eprintln!("[lzc-sync] account mapping: {account_mapping:?}");
    MAPPING.set(account_mapping).ok();

    // 启动代理（先绑定端口获取实际端口号，再启动服务线程）
    let (server, port) = match proxy::start(UPSTREAM_URL) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[lzc-sync] failed to start proxy: {e}");
            unsafe {
                REAL_MAIN = Some(main);
                return real_start_main(wrapped_main, argc, argv, init, fini, rtld_fini, stack_end);
            }
        }
    };

    let upstream = UPSTREAM_URL.to_string();
    std::thread::spawn(move || {
        if let Err(e) = proxy::run(server, &upstream) {
            eprintln!("[lzc-sync] proxy error: {e}");
        }
    });

    // 注入 --sync-url
    let sync_url_arg =
        CString::new(format!("--sync-url=http://127.0.0.1:{port}/chrome-sync")).unwrap();

    let new_argc = argc + 1;
    let mut new_argv: Vec<*mut c_char> = Vec::with_capacity(new_argc as usize + 1);
    for i in 0..argc as isize {
        unsafe {
            new_argv.push(*argv.offset(i));
        }
    }
    new_argv.push(sync_url_arg.into_raw());
    new_argv.push(std::ptr::null_mut());

    unsafe {
        REAL_MAIN = Some(main);
        real_start_main(
            wrapped_main,
            new_argc,
            new_argv.as_mut_ptr(),
            init,
            fini,
            rtld_fini,
            stack_end,
        )
    }
}
