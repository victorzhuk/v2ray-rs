//! Raw IP socket sender: re-injects generated packets with `IP_HDRINCL`,
//! carrying `SO_MARK` fwmark so the queue rules exclude them (loop guard),
//! matching nfqws' raw-socket design.

use crate::error::{Error, Result};
use std::net::Ipv4Addr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

pub struct RawSender {
    fd: OwnedFd,
}

impl RawSender {
    pub fn new(mark: u32) -> Result<RawSender> {
        unsafe {
            let fd = libc::socket(
                libc::AF_INET,
                libc::SOCK_RAW | libc::SOCK_CLOEXEC,
                libc::IPPROTO_RAW,
            );
            if fd < 0 {
                return Err(Error::RawSend(format!(
                    "socket(AF_INET, SOCK_RAW, IPPROTO_RAW): {} (needs cap_net_raw)",
                    std::io::Error::last_os_error()
                )));
            }
            let fd = OwnedFd::from_raw_fd(fd);
            let one: libc::c_int = 1;
            if libc::setsockopt(
                fd.as_raw_fd(),
                libc::IPPROTO_IP,
                libc::IP_HDRINCL,
                &one as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as u32,
            ) < 0
            {
                return Err(Error::RawSend(format!(
                    "IP_HDRINCL: {err}",
                    err = std::io::Error::last_os_error()
                )));
            }
            if mark != 0
                && libc::setsockopt(
                    fd.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_MARK,
                    &mark as *const u32 as *const libc::c_void,
                    std::mem::size_of::<u32>() as u32,
                ) < 0
            {
                return Err(Error::RawSend(format!(
                    "SO_MARK: {err} (needs cap_net_admin)",
                    err = std::io::Error::last_os_error()
                )));
            }
            Ok(RawSender { fd })
        }
    }

    pub fn send(&self, bytes: &[u8], dst: Ipv4Addr) -> Result<usize> {
        let addr = libc::sockaddr_in {
            sin_family: libc::AF_INET as libc::sa_family_t,
            sin_port: 0,
            sin_addr: libc::in_addr {
                s_addr: u32::from_ne_bytes(dst.octets()),
            },
            sin_zero: [0; 8],
        };
        let sent = unsafe {
            libc::sendto(
                self.fd.as_raw_fd(),
                bytes.as_ptr() as *const libc::c_void,
                bytes.len(),
                0,
                &addr as *const libc::sockaddr_in as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in>() as u32,
            )
        };
        if sent < 0 {
            return Err(Error::RawSend(format!(
                "sendto: {err}",
                err = std::io::Error::last_os_error()
            )));
        }
        Ok(sent as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_requires_privilege_or_works() {
        // In a privileged test env this succeeds; unprivileged CI gets a
        // clean error rather than a panic.
        match RawSender::new(0x40000000) {
            Ok(_) => {}
            Err(e) => assert!(matches!(e, Error::RawSend(_))),
        }
    }
}
