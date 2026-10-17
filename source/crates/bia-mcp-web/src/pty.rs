//! Pseudo-terminal for root's web shell (`/ws/shell`).
use std::os::fd::RawFd;

pub fn open_pty() -> Result<(RawFd, RawFd), String> {
    unsafe {
        #[cfg(target_os = "linux")]
        {
            let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            if master < 0 {
                return Err(format!("posix_openpt: {}", std::io::Error::last_os_error()));
            }
            if libc::grantpt(master) != 0 {
                let e = std::io::Error::last_os_error();
                libc::close(master);
                return Err(format!("grantpt: {e}"));
            }
            if libc::unlockpt(master) != 0 {
                let e = std::io::Error::last_os_error();
                libc::close(master);
                return Err(format!("unlockpt: {e}"));
            }
            let mut buf = [0 as libc::c_char; 128];
            if libc::ptsname_r(master, buf.as_mut_ptr(), buf.len()) != 0 {
                let e = std::io::Error::last_os_error();
                libc::close(master);
                return Err(format!("ptsname_r: {e}"));
            }
            let slave = libc::open(buf.as_ptr(), libc::O_RDWR | libc::O_NOCTTY);
            if slave < 0 {
                let e = std::io::Error::last_os_error();
                libc::close(master);
                return Err(format!("open pts: {e}"));
            }
            let ws = libc::winsize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            let _ = libc::ioctl(master, libc::TIOCSWINSZ, &ws);
            Ok((master, slave))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let mut master = 0;
            let mut slave = 0;
            if libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) != 0
            {
                return Err(format!("openpty: {}", std::io::Error::last_os_error()));
            }
            let ws = libc::winsize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            let _ = libc::ioctl(master, libc::TIOCSWINSZ, &ws);
            Ok((master, slave))
        }
    }
}
