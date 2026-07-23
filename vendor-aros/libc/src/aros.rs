//! AROS (hosted, darwin-aarch64) libc bindings.
//!
//! AROS is not `cfg(unix)`, so the `libc` crate provides it an empty module by
//! default, which leaves the async ecosystem (rustix → async-io/mio) unable to
//! compile. This module supplies the surface those crates use. AROS's socket
//! layer is FreeBSD-derived (BSD `sa_len` sockaddrs, `SO_*` bit flags,
//! `SOL_SOCKET = 0xffff`), so the shapes mirror the BSD definitions; the
//! constant values are taken from the AROS SDK headers.
//!
//! Phase 1: primitive/socket types, errno, socket + IP constants, the core
//! sockaddr structs, and the net/fd entry points. `fs`/`ioctl`/`pthread` and
//! the remaining rustix surface are added incrementally.

use crate::prelude::*;

// ---- primitive aliases (c_char/c_int/... come from the shared prelude) -----

pub type size_t = usize;
pub type ssize_t = isize;
pub type ptrdiff_t = isize;
pub type intptr_t = isize;
pub type uintptr_t = usize;
pub type wchar_t = i32;

pub type mode_t = u16;
pub type off_t = i64;
pub type pid_t = i32;
pub type uid_t = u32;
pub type gid_t = u32;
pub type socklen_t = u32;
pub type sa_family_t = u8;
pub type in_port_t = u16;
pub type in_addr_t = u32;
pub type nfds_t = c_ulong;
pub type time_t = i32;
pub type suseconds_t = i64;

// filesystem types (aros/types/*; __USE_FILE_OFFSET64 is NOTIMPL on AROS, so
// the default non-LFS typedefs are authoritative).
pub type dev_t = u64; // unsigned AROS_INTPTR_TYPE
pub type ino_t = i32; // signed AROS_32BIT_TYPE (default, non-LFS)
pub type nlink_t = u16; // unsigned AROS_16BIT_TYPE
pub type blksize_t = i64; // signed AROS_64BIT_TYPE (__WORDSIZE==64)
pub type blkcnt_t = i64; // signed AROS_64BIT_TYPE (__WORDSIZE==64)
pub type fsblkcnt_t = u64; // AROS_64BIT_TYPE (default, non-LFS on aarch64)
pub type fsfilcnt_t = u64;

// ---- errno (from AROS <sys/errno.h>) ---------------------------------------

pub const EPERM: c_int = 1;
pub const ENOENT: c_int = 2;
pub const ESRCH: c_int = 3;
pub const EINTR: c_int = 4;
pub const EIO: c_int = 5;
pub const ENXIO: c_int = 6;
pub const E2BIG: c_int = 7;
pub const ENOEXEC: c_int = 8;
pub const EBADF: c_int = 9;
pub const ECHILD: c_int = 10;
pub const EDEADLK: c_int = 11;
pub const ENOMEM: c_int = 12;
pub const EACCES: c_int = 13;
pub const EFAULT: c_int = 14;
pub const ENOTBLK: c_int = 15;
pub const EBUSY: c_int = 16;
pub const EEXIST: c_int = 17;
pub const EXDEV: c_int = 18;
pub const ENODEV: c_int = 19;
pub const ENOTDIR: c_int = 20;
pub const EISDIR: c_int = 21;
pub const EINVAL: c_int = 22;
pub const ENFILE: c_int = 23;
pub const EMFILE: c_int = 24;
pub const ENOTTY: c_int = 25;
pub const ETXTBSY: c_int = 26;
pub const EFBIG: c_int = 27;
pub const ENOSPC: c_int = 28;
pub const ESPIPE: c_int = 29;
pub const EROFS: c_int = 30;
pub const EMLINK: c_int = 31;
pub const EPIPE: c_int = 32;
pub const EDOM: c_int = 33;
pub const ERANGE: c_int = 34;
pub const EAGAIN: c_int = 35;
pub const EWOULDBLOCK: c_int = EAGAIN;
pub const EINPROGRESS: c_int = 36;
pub const EALREADY: c_int = 37;
pub const ENOTSOCK: c_int = 38;
pub const EDESTADDRREQ: c_int = 39;
pub const EMSGSIZE: c_int = 40;
pub const EPROTOTYPE: c_int = 41;
pub const ENOPROTOOPT: c_int = 42;
pub const EPROTONOSUPPORT: c_int = 43;
pub const ESOCKTNOSUPPORT: c_int = 44;
pub const EOPNOTSUPP: c_int = 45;
pub const ENOTSUP: c_int = EOPNOTSUPP;
pub const EPFNOSUPPORT: c_int = 46;
pub const EAFNOSUPPORT: c_int = 47;
pub const EADDRINUSE: c_int = 48;
pub const EADDRNOTAVAIL: c_int = 49;
pub const ENETDOWN: c_int = 50;
pub const ENETUNREACH: c_int = 51;
pub const ENETRESET: c_int = 52;
pub const ECONNABORTED: c_int = 53;
pub const ECONNRESET: c_int = 54;
pub const ENOBUFS: c_int = 55;
pub const EISCONN: c_int = 56;
pub const ENOTCONN: c_int = 57;
pub const ESHUTDOWN: c_int = 58;
pub const ETOOMANYREFS: c_int = 59;
pub const ETIMEDOUT: c_int = 60;
pub const ECONNREFUSED: c_int = 61;
pub const ELOOP: c_int = 62;
pub const ENAMETOOLONG: c_int = 63;
pub const EHOSTDOWN: c_int = 64;
pub const EHOSTUNREACH: c_int = 65;
pub const ENOTEMPTY: c_int = 66;
pub const ENOLCK: c_int = 77;
pub const ENOSYS: c_int = 78;

// ---- socket constants (from AROS <sys/socket.h>) ---------------------------

pub const SOCK_STREAM: c_int = 1;
pub const SOCK_DGRAM: c_int = 2;
pub const SOCK_RAW: c_int = 3;
pub const SOCK_RDM: c_int = 4;
pub const SOCK_SEQPACKET: c_int = 5;
pub const SOCK_CLOEXEC: c_int = 0x1000_0000;
pub const SOCK_NONBLOCK: c_int = 0x2000_0000;

pub const SOL_SOCKET: c_int = 0xffff;

pub const SO_DEBUG: c_int = 0x0001;
pub const SO_ACCEPTCONN: c_int = 0x0002;
pub const SO_REUSEADDR: c_int = 0x0004;
pub const SO_KEEPALIVE: c_int = 0x0008;
pub const SO_DONTROUTE: c_int = 0x0010;
pub const SO_BROADCAST: c_int = 0x0020;
pub const SO_USELOOPBACK: c_int = 0x0040;
pub const SO_LINGER: c_int = 0x0080;
pub const SO_OOBINLINE: c_int = 0x0100;
pub const SO_REUSEPORT: c_int = 0x0200;
pub const SO_TIMESTAMP: c_int = 0x0400;
pub const SO_SNDBUF: c_int = 0x1001;
pub const SO_RCVBUF: c_int = 0x1002;
pub const SO_SNDLOWAT: c_int = 0x1003;
pub const SO_RCVLOWAT: c_int = 0x1004;
pub const SO_SNDTIMEO: c_int = 0x1005;
pub const SO_RCVTIMEO: c_int = 0x1006;
pub const SO_ERROR: c_int = 0x1007;
pub const SO_TYPE: c_int = 0x1008;

pub const AF_UNSPEC: c_int = 0;
pub const AF_UNIX: c_int = 1;
pub const AF_INET: c_int = 2;
pub const AF_INET6: c_int = 28;
pub const PF_UNSPEC: c_int = AF_UNSPEC;
pub const PF_UNIX: c_int = AF_UNIX;
pub const PF_INET: c_int = AF_INET;
pub const PF_INET6: c_int = AF_INET6;

pub const SOMAXCONN: c_int = 128;

pub const MSG_OOB: c_int = 0x0001;
pub const MSG_PEEK: c_int = 0x0002;
pub const MSG_DONTROUTE: c_int = 0x0004;
pub const MSG_EOR: c_int = 0x0008;
pub const MSG_TRUNC: c_int = 0x0010;
pub const MSG_CTRUNC: c_int = 0x0020;
pub const MSG_WAITALL: c_int = 0x0040;
pub const MSG_DONTWAIT: c_int = 0x0080;
pub const MSG_NOSIGNAL: c_int = 0x0002_0000;

pub const SHUT_RD: c_int = 0;
pub const SHUT_WR: c_int = 1;
pub const SHUT_RDWR: c_int = 2;

// ---- IP protocols (from AROS <netinet/in.h>) -------------------------------

pub const IPPROTO_IP: c_int = 0;
pub const IPPROTO_ICMP: c_int = 1;
pub const IPPROTO_TCP: c_int = 6;
pub const IPPROTO_UDP: c_int = 17;
pub const IPPROTO_IPV6: c_int = 41;
pub const IPPROTO_ICMPV6: c_int = 58;
pub const IPPROTO_RAW: c_int = 255;

pub const INADDR_ANY: in_addr_t = 0;
pub const INADDR_LOOPBACK: in_addr_t = 0x7f00_0001;
pub const INADDR_BROADCAST: in_addr_t = 0xffff_ffff;
pub const INADDR_NONE: in_addr_t = 0xffff_ffff;

// ---- fd / fcntl ------------------------------------------------------------

pub const STDIN_FILENO: c_int = 0;
pub const STDOUT_FILENO: c_int = 1;
pub const STDERR_FILENO: c_int = 2;

pub const F_DUPFD: c_int = 0;
pub const F_DUPFD_CLOEXEC: c_int = 1;
pub const F_GETFD: c_int = 2;
pub const F_SETFD: c_int = 3;
pub const F_GETFL: c_int = 4;
pub const F_SETFL: c_int = 5;
pub const FD_CLOEXEC: c_int = 1;

pub const O_RDONLY: c_int = 0x0001;
pub const O_WRONLY: c_int = 0x0002;
pub const O_RDWR: c_int = 0x0003;
pub const O_ACCMODE: c_int = 0x0003;
pub const O_EXEC: c_int = 0x0004;
pub const O_CREAT: c_int = 0x0040;
pub const O_EXCL: c_int = 0x0080;
pub const O_NOCTTY: c_int = 0;
pub const O_TRUNC: c_int = 0x0200;
pub const O_APPEND: c_int = 0x0400;
pub const O_NONBLOCK: c_int = 0x0800;
pub const O_SYNC: c_int = 0x1000;
pub const O_ASYNC: c_int = 0x2000;
pub const O_DSYNC: c_int = 0x4000;
pub const O_CLOEXEC: c_int = 0x1_0000;
pub const O_DIRECTORY: c_int = 0x2_0000;
pub const O_NOFOLLOW: c_int = 0x4_0000;

// ---- structs ---------------------------------------------------------------

s! {
    pub struct in_addr {
        pub s_addr: in_addr_t,
    }

    pub struct in6_addr {
        pub s6_addr: [u8; 16],
    }

    pub struct sockaddr {
        pub sa_len: u8,
        pub sa_family: sa_family_t,
        pub sa_data: [c_char; 14],
    }

    pub struct sockaddr_in {
        pub sin_len: u8,
        pub sin_family: sa_family_t,
        pub sin_port: in_port_t,
        pub sin_addr: in_addr,
        pub sin_zero: [c_char; 8],
    }

    pub struct sockaddr_in6 {
        pub sin6_len: u8,
        pub sin6_family: sa_family_t,
        pub sin6_port: in_port_t,
        pub sin6_flowinfo: u32,
        pub sin6_addr: in6_addr,
        pub sin6_scope_id: u32,
    }

    pub struct sockaddr_storage {
        pub ss_len: u8,
        pub ss_family: sa_family_t,
        __ss_pad1: [u8; 6],
        __ss_align: i64,
        __ss_pad2: [u8; 112],
    }

    pub struct iovec {
        pub iov_base: *mut c_void,
        pub iov_len: size_t,
    }

    pub struct msghdr {
        pub msg_name: *mut c_void,
        pub msg_namelen: socklen_t,
        pub msg_iov: *mut iovec,
        pub msg_iovlen: c_int,
        pub msg_control: *mut c_void,
        pub msg_controllen: socklen_t,
        pub msg_flags: c_int,
    }

    pub struct linger {
        pub l_onoff: c_int,
        pub l_linger: c_int,
    }

    pub struct timeval {
        pub tv_sec: time_t,
        pub tv_usec: suseconds_t,
    }

    // aros/types/timespec_s.h: { time_t tv_sec; long tv_nsec; }
    // On aarch64 the C compiler pads tv_sec (i32) to 8 before the long,
    // which a repr(C) { i32, i64 } reproduces exactly.
    pub struct timespec {
        pub tv_sec: time_t,
        pub tv_nsec: c_long,
    }

    // aros/posixc/sys/stat.h, default (non-LFS) branch. Field widths and the
    // aarch64 natural padding give sizeof == 120, align 8.
    pub struct stat {
        pub st_dev: dev_t,
        pub st_ino: ino_t,
        pub st_mode: mode_t,
        pub st_nlink: nlink_t,
        pub st_uid: uid_t,
        pub st_gid: gid_t,
        pub st_rdev: dev_t,
        pub st_size: off_t,
        pub st_atim: timespec,
        pub st_mtim: timespec,
        pub st_ctim: timespec,
        pub st_blksize: blksize_t,
        pub st_blocks: blkcnt_t,
        pub st_flags: c_ulong,
        pub st_gen: c_ulong,
    }

    // sys/socket.h
    pub struct cmsghdr {
        pub cmsg_len: socklen_t,
        pub cmsg_level: c_int,
        pub cmsg_type: c_int,
    }

    // netinet/in.h
    pub struct ip_mreq {
        pub imr_multiaddr: in_addr,
        pub imr_interface: in_addr,
    }

    pub struct ipv6_mreq {
        pub ipv6mr_multiaddr: in6_addr,
        pub ipv6mr_interface: c_uint,
    }

    // sys/mount.h
    pub struct fsid_t {
        pub val: [i32; 2],
    }

    // aros/posixc/sys/statvfs.h (all fields 64-bit on aarch64)
    pub struct statvfs {
        pub f_bsize: c_ulong,
        pub f_frsize: c_ulong,
        pub f_blocks: fsblkcnt_t,
        pub f_bfree: fsblkcnt_t,
        pub f_bavail: fsblkcnt_t,
        pub f_files: fsfilcnt_t,
        pub f_ffree: fsfilcnt_t,
        pub f_favail: fsfilcnt_t,
        pub f_fsid: c_ulong,
        pub f_flag: c_ulong,
        pub f_namemax: c_ulong,
    }

    // aros/posixc/sys/mount.h (legacy BSD statfs; MNAMELEN == 90)
    pub struct statfs {
        pub f_type: c_short,
        pub f_flags: c_short,
        pub f_fsize: c_long,
        pub f_bsize: c_long,
        pub f_blocks: c_long,
        pub f_bfree: c_long,
        pub f_bavail: c_long,
        pub f_files: c_long,
        pub f_ffree: c_long,
        pub f_fsid: fsid_t,
        pub f_spare: [c_long; 9],
        pub f_mntonname: [c_char; 90],
        pub f_mntfromname: [c_char; 90],
    }

    // aros/posixc/dirent.h, default (non-LFS) branch. d_name is declared
    // char[PATH_MAX + 1]; only the offset matters to callers that walk by
    // d_reclen, so a nominal length is used for the trailing array.
    pub struct dirent {
        pub d_fileno: ino_t,
        pub d_off: off_t,
        pub d_reclen: c_ushort,
        pub d_type: c_uchar,
        pub d_name: [c_char; 1024],
    }
}

// Opaque directory stream handle (readdir family operates through a pointer).
#[allow(missing_debug_implementations, missing_copy_implementations)]
pub enum DIR {}

// dlsym "default" handle sentinel (no real dlfcn on AROS; null is a safe stand-in).
pub const RTLD_DEFAULT: *mut c_void = core::ptr::null_mut();

// ---- filesystem + extended syscalls (posixc) -------------------------------
extern "C" {
    pub fn open(path: *const c_char, oflag: c_int, ...) -> c_int;
    pub fn openat(dirfd: c_int, path: *const c_char, oflag: c_int, ...) -> c_int;
    pub fn stat(path: *const c_char, buf: *mut stat) -> c_int;
    pub fn fstat(fd: c_int, buf: *mut stat) -> c_int;
    pub fn lstat(path: *const c_char, buf: *mut stat) -> c_int;
    pub fn fstatat(dirfd: c_int, path: *const c_char, buf: *mut stat, flags: c_int) -> c_int;
    pub fn access(path: *const c_char, amode: c_int) -> c_int;
    pub fn faccessat(dirfd: c_int, path: *const c_char, amode: c_int, flags: c_int) -> c_int;
    pub fn chmod(path: *const c_char, mode: mode_t) -> c_int;
    pub fn fchmod(fd: c_int, mode: mode_t) -> c_int;
    pub fn fchmodat(dirfd: c_int, path: *const c_char, mode: mode_t, flags: c_int) -> c_int;
    pub fn chown(path: *const c_char, owner: uid_t, group: gid_t) -> c_int;
    pub fn fchown(fd: c_int, owner: uid_t, group: gid_t) -> c_int;
    pub fn fchownat(
        dirfd: c_int,
        path: *const c_char,
        owner: uid_t,
        group: gid_t,
        flags: c_int,
    ) -> c_int;
    pub fn lseek(fd: c_int, offset: off_t, whence: c_int) -> off_t;
    pub fn ftruncate(fd: c_int, length: off_t) -> c_int;
    pub fn fsync(fd: c_int) -> c_int;
    pub fn fdatasync(fd: c_int) -> c_int;
    pub fn sync();
    pub fn link(src: *const c_char, dst: *const c_char) -> c_int;
    pub fn linkat(
        olddirfd: c_int,
        oldpath: *const c_char,
        newdirfd: c_int,
        newpath: *const c_char,
        flags: c_int,
    ) -> c_int;
    pub fn unlink(path: *const c_char) -> c_int;
    pub fn unlinkat(dirfd: c_int, path: *const c_char, flags: c_int) -> c_int;
    pub fn rename(oldpath: *const c_char, newpath: *const c_char) -> c_int;
    pub fn renameat(
        olddirfd: c_int,
        oldpath: *const c_char,
        newdirfd: c_int,
        newpath: *const c_char,
    ) -> c_int;
    pub fn mkdir(path: *const c_char, mode: mode_t) -> c_int;
    pub fn mkdirat(dirfd: c_int, path: *const c_char, mode: mode_t) -> c_int;
    pub fn mknodat(dirfd: c_int, path: *const c_char, mode: mode_t, dev: dev_t) -> c_int;
    pub fn rmdir(path: *const c_char) -> c_int;
    pub fn symlink(target: *const c_char, linkpath: *const c_char) -> c_int;
    pub fn symlinkat(target: *const c_char, newdirfd: c_int, linkpath: *const c_char) -> c_int;
    pub fn readlink(path: *const c_char, buf: *mut c_char, bufsz: size_t) -> ssize_t;
    pub fn readlinkat(
        dirfd: c_int,
        path: *const c_char,
        buf: *mut c_char,
        bufsz: size_t,
    ) -> ssize_t;
    pub fn dup2(oldfd: c_int, newfd: c_int) -> c_int;
    pub fn dup3(oldfd: c_int, newfd: c_int, flags: c_int) -> c_int;
    pub fn pread(fd: c_int, buf: *mut c_void, count: size_t, offset: off_t) -> ssize_t;
    pub fn pwrite(fd: c_int, buf: *const c_void, count: size_t, offset: off_t) -> ssize_t;
    pub fn readv(fd: c_int, iov: *const iovec, iovcnt: c_int) -> ssize_t;
    pub fn writev(fd: c_int, iov: *const iovec, iovcnt: c_int) -> ssize_t;
    pub fn preadv(fd: c_int, iov: *const iovec, iovcnt: c_int, offset: off_t) -> ssize_t;
    pub fn pwritev(fd: c_int, iov: *const iovec, iovcnt: c_int, offset: off_t) -> ssize_t;
    pub fn sendmsg(fd: c_int, msg: *const msghdr, flags: c_int) -> ssize_t;
    pub fn recvmsg(fd: c_int, msg: *mut msghdr, flags: c_int) -> ssize_t;
    pub fn socketpair(domain: c_int, ty: c_int, proto: c_int, sv: *mut c_int) -> c_int;
    pub fn accept4(
        fd: c_int,
        addr: *mut sockaddr,
        len: *mut socklen_t,
        flags: c_int,
    ) -> c_int;
    pub fn flock(fd: c_int, operation: c_int) -> c_int;
    pub fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    pub fn getuid() -> uid_t;
    pub fn geteuid() -> uid_t;
    pub fn getgid() -> gid_t;
    pub fn getegid() -> gid_t;
    pub fn futimens(fd: c_int, times: *const timespec) -> c_int;
    pub fn utimensat(
        dirfd: c_int,
        path: *const c_char,
        times: *const timespec,
        flags: c_int,
    ) -> c_int;
    pub fn posix_fadvise(fd: c_int, offset: off_t, len: off_t, advice: c_int) -> c_int;
    pub fn posix_fallocate(fd: c_int, offset: off_t, len: off_t) -> c_int;
    pub fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    pub fn closedir(dirp: *mut DIR) -> c_int;
    pub fn fdopendir(fd: c_int) -> *mut DIR;
    pub fn rewinddir(dirp: *mut DIR);
    pub fn seekdir(dirp: *mut DIR, loc: c_long);
    pub fn dirfd(dirp: *mut DIR) -> c_int;
    pub fn opendir(path: *const c_char) -> *mut DIR;
    pub fn readdir(dirp: *mut DIR) -> *mut dirent;
    pub fn statvfs(path: *const c_char, buf: *mut statvfs) -> c_int;
    pub fn fstatvfs(fd: c_int, buf: *mut statvfs) -> c_int;
    pub fn statfs(path: *const c_char, buf: *mut statfs) -> c_int;
    pub fn fstatfs(fd: c_int, buf: *mut statfs) -> c_int;
}

// Device-number helpers. AROS device numbers are nominal; this uses the
// conventional 8-bit minor split so the values round-trip.
pub const fn major(dev: dev_t) -> c_uint {
    ((dev >> 8) & 0xff) as c_uint
}
pub const fn minor(dev: dev_t) -> c_uint {
    (dev & 0xff) as c_uint
}
pub const fn makedev(major: c_uint, minor: c_uint) -> dev_t {
    (((major & 0xff) << 8) | (minor & 0xff)) as dev_t
}

// ---- IPv4 option names (netinet/in.h) --------------------------------------
pub const IP_TOS: c_int = 3;
pub const IP_TTL: c_int = 4;
pub const IP_MULTICAST_IF: c_int = 9;
pub const IP_MULTICAST_TTL: c_int = 10;
pub const IP_MULTICAST_LOOP: c_int = 11;
pub const IP_ADD_MEMBERSHIP: c_int = 12;
pub const IP_DROP_MEMBERSHIP: c_int = 13;

// ---- dirent d_type values (dirent.h) ---------------------------------------
pub const DT_UNKNOWN: c_uchar = 0;
pub const DT_FIFO: c_uchar = 1;
pub const DT_CHR: c_uchar = 2;
pub const DT_DIR: c_uchar = 4;
pub const DT_BLK: c_uchar = 6;
pub const DT_REG: c_uchar = 8;
pub const DT_LNK: c_uchar = 10;
pub const DT_SOCK: c_uchar = 12;
pub const DT_WHT: c_uchar = 14;

// fcntl record lock. AROS has no struct flock in its headers; rustix needs the
// type to build its F_SETLK path, so the standard BSD layout is provided.
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct flock {
    pub l_start: off_t,
    pub l_len: off_t,
    pub l_pid: pid_t,
    pub l_type: c_short,
    pub l_whence: c_short,
}

// Control-message helpers (sys/socket.h); _ALIGN rounds to sizeof(long) == 8.
const fn cmsg_align(len: usize) -> usize {
    let a = core::mem::size_of::<c_long>();
    (len + a - 1) & !(a - 1)
}
pub fn CMSG_DATA(cmsg: *const cmsghdr) -> *mut c_uchar {
    unsafe { (cmsg as *mut c_uchar).add(cmsg_align(core::mem::size_of::<cmsghdr>())) }
}
pub const fn CMSG_LEN(length: c_uint) -> c_uint {
    (cmsg_align(core::mem::size_of::<cmsghdr>()) + length as usize) as c_uint
}
pub const fn CMSG_SPACE(length: c_uint) -> c_uint {
    (cmsg_align(core::mem::size_of::<cmsghdr>()) + cmsg_align(length as usize)) as c_uint
}
pub fn CMSG_FIRSTHDR(mhdr: *const msghdr) -> *mut cmsghdr {
    unsafe {
        if (*mhdr).msg_controllen as usize >= core::mem::size_of::<cmsghdr>() {
            (*mhdr).msg_control as *mut cmsghdr
        } else {
            core::ptr::null_mut()
        }
    }
}
pub fn CMSG_NXTHDR(mhdr: *const msghdr, cmsg: *const cmsghdr) -> *mut cmsghdr {
    unsafe {
        if ((*cmsg).cmsg_len as usize) < core::mem::size_of::<cmsghdr>() {
            return core::ptr::null_mut();
        }
        let next = (cmsg as usize + cmsg_align((*cmsg).cmsg_len as usize)) as *mut cmsghdr;
        let max = (*mhdr).msg_control as usize + (*mhdr).msg_controllen as usize;
        if next.add(1) as usize > max {
            core::ptr::null_mut()
        } else {
            next
        }
    }
}

// ---- remaining protocol / socket / message constants -----------------------
// Real AROS values (netinet/in.h) where present; Linux-only names AROS lacks
// are given their conventional values as placeholders.
pub const IPPROTO_IGMP: c_int = 2;
pub const IPPROTO_EGP: c_int = 8;
pub const IPPROTO_PUP: c_int = 12;
pub const IPPROTO_IDP: c_int = 22;
pub const IPPROTO_TP: c_int = 29;
pub const IPPROTO_ROUTING: c_int = 43;
pub const IPPROTO_FRAGMENT: c_int = 44;
pub const IPPROTO_RSVP: c_int = 46;
pub const IPPROTO_GRE: c_int = 47;
pub const IPPROTO_ESP: c_int = 50;
pub const IPPROTO_AH: c_int = 51;
pub const IPPROTO_SCTP: c_int = 132;
pub const IPPROTO_UDPLITE: c_int = 136;
pub const IPPROTO_DCCP: c_int = 33;
pub const IPPROTO_IPIP: c_int = 4;
pub const IPPROTO_MTP: c_int = 92;
pub const IPPROTO_BEETPH: c_int = 94;
pub const IPPROTO_ENCAP: c_int = 98;
pub const IPPROTO_PIM: c_int = 103;
pub const IPPROTO_COMP: c_int = 108;
pub const IPPROTO_MH: c_int = 135;
pub const IPPROTO_MPLS: c_int = 137;
pub const IPPROTO_MPTCP: c_int = 262;

pub const IPV6_ADD_MEMBERSHIP: c_int = 12; // IPV6_JOIN_GROUP
pub const IPV6_DROP_MEMBERSHIP: c_int = 13; // IPV6_LEAVE_GROUP
pub const TCP_KEEPIDLE: c_int = 256;
pub const SO_DOMAIN: c_int = 0x1019;

pub const MSG_CMSG_CLOEXEC: c_int = 0x0004_0000;
// Linux-only message flags AROS lacks (placeholders above AROS's used range).
pub const MSG_CONFIRM: c_int = 0x0080_0000;
pub const MSG_ERRQUEUE: c_int = 0x0100_0000;
pub const MSG_MORE: c_int = 0x0200_0000;

// ---- functions (posixc / bsdsocket linklib) --------------------------------

unsafe extern "C" {
    pub fn __stdc_geterrnoptr() -> *mut c_int;

    pub fn close(fd: c_int) -> c_int;
    pub fn dup(fd: c_int) -> c_int;
    pub fn read(fd: c_int, buf: *mut c_void, count: size_t) -> ssize_t;
    pub fn write(fd: c_int, buf: *const c_void, count: size_t) -> ssize_t;
    pub fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    pub fn strerror_r(errnum: c_int, buf: *mut c_char, buflen: size_t) -> c_int;
    pub fn strlen(cs: *const c_char) -> size_t;

    pub fn socket(domain: c_int, ty: c_int, protocol: c_int) -> c_int;
    pub fn bind(fd: c_int, addr: *const sockaddr, len: socklen_t) -> c_int;
    pub fn connect(fd: c_int, addr: *const sockaddr, len: socklen_t) -> c_int;
    pub fn listen(fd: c_int, backlog: c_int) -> c_int;
    pub fn accept(fd: c_int, addr: *mut sockaddr, len: *mut socklen_t) -> c_int;
    pub fn getsockname(fd: c_int, addr: *mut sockaddr, len: *mut socklen_t) -> c_int;
    pub fn getpeername(fd: c_int, addr: *mut sockaddr, len: *mut socklen_t) -> c_int;
    pub fn send(fd: c_int, buf: *const c_void, len: size_t, flags: c_int) -> ssize_t;
    pub fn recv(fd: c_int, buf: *mut c_void, len: size_t, flags: c_int) -> ssize_t;
    pub fn sendto(
        fd: c_int,
        buf: *const c_void,
        len: size_t,
        flags: c_int,
        addr: *const sockaddr,
        addrlen: socklen_t,
    ) -> ssize_t;
    pub fn recvfrom(
        fd: c_int,
        buf: *mut c_void,
        len: size_t,
        flags: c_int,
        addr: *mut sockaddr,
        addrlen: *mut socklen_t,
    ) -> ssize_t;
    pub fn setsockopt(
        fd: c_int,
        level: c_int,
        name: c_int,
        value: *const c_void,
        len: socklen_t,
    ) -> c_int;
    pub fn getsockopt(
        fd: c_int,
        level: c_int,
        name: c_int,
        value: *mut c_void,
        len: *mut socklen_t,
    ) -> c_int;
    pub fn shutdown(fd: c_int, how: c_int) -> c_int;
}

// ===========================================================================
// Phase 2: filesystem + extended constants that rustix references.
//
// Values are taken from the AROS SDK headers where AROS defines them. The
// errno and address-family names below that AROS does NOT provide (Linux/exotic
// families and errnos) are given distinct placeholder values so that rustix's
// tables resolve; AROS never returns them.
// ===========================================================================

// ---- POSIX errnos AROS defines (values from posixc/errno.h) ----------------
pub const EBADMSG: c_int = 88;
pub const ECANCELED: c_int = 87;
pub const EDQUOT: c_int = 69;
pub const EIDRM: c_int = 82;
pub const EMULTIHOP: c_int = 94;
pub const ENODATA: c_int = 89;
pub const ENOLINK: c_int = 95;
pub const ENOMSG: c_int = 83;
pub const ENOSR: c_int = 90;
pub const ENOSTR: c_int = 91;
pub const EOVERFLOW: c_int = 84;
pub const EPROTO: c_int = 96;
pub const ESTALE: c_int = 70;
pub const ETIME: c_int = 92;
pub const EUSERS: c_int = 68;

// ---- errnos AROS does not have (placeholders, never returned) --------------
pub const EADV: c_int = 200;
pub const EBADE: c_int = 201;
pub const EBADFD: c_int = 202;
pub const EBADR: c_int = 203;
pub const EBADRQC: c_int = 204;
pub const EBADSLT: c_int = 205;
pub const EBFONT: c_int = 206;
pub const ECHRNG: c_int = 207;
pub const ECOMM: c_int = 208;
pub const EDEADLOCK: c_int = 209;
pub const EDOTDOT: c_int = 210;
pub const EHWPOISON: c_int = 211;
pub const EILSEQ: c_int = 212;
pub const EISNAM: c_int = 213;
pub const EKEYEXPIRED: c_int = 214;
pub const EKEYREJECTED: c_int = 215;
pub const EKEYREVOKED: c_int = 216;
pub const EL2HLT: c_int = 217;
pub const EL2NSYNC: c_int = 218;
pub const EL3HLT: c_int = 219;
pub const EL3RST: c_int = 220;
pub const ELIBACC: c_int = 221;
pub const ELIBBAD: c_int = 222;
pub const ELIBEXEC: c_int = 223;
pub const ELIBMAX: c_int = 224;
pub const ELIBSCN: c_int = 225;
pub const ELNRNG: c_int = 226;
pub const EMEDIUMTYPE: c_int = 227;
pub const ENAVAIL: c_int = 228;
pub const ENOANO: c_int = 229;
pub const ENOCSI: c_int = 230;
pub const ENOKEY: c_int = 231;
pub const ENOMEDIUM: c_int = 232;
pub const ENONET: c_int = 233;
pub const ENOPKG: c_int = 234;
pub const ENOTNAM: c_int = 235;
pub const ENOTRECOVERABLE: c_int = 236;
pub const ENOTUNIQ: c_int = 237;
pub const EOWNERDEAD: c_int = 238;
pub const EREMCHG: c_int = 239;
pub const EREMOTE: c_int = 240;
pub const EREMOTEIO: c_int = 241;
pub const ERESTART: c_int = 242;
pub const ERFKILL: c_int = 243;
pub const ESRMNT: c_int = 244;
pub const ESTRPIPE: c_int = 245;
pub const EUCLEAN: c_int = 246;
pub const EUNATCH: c_int = 247;
pub const EXFULL: c_int = 248;

// ---- address families AROS does not have (placeholders) --------------------
pub const AF_APPLETALK: c_int = 100;
pub const AF_ASH: c_int = 101;
pub const AF_ATMPVC: c_int = 102;
pub const AF_ATMSVC: c_int = 103;
pub const AF_AX25: c_int = 104;
pub const AF_BLUETOOTH: c_int = 105;
pub const AF_BRIDGE: c_int = 106;
pub const AF_CAN: c_int = 107;
pub const AF_DECnet: c_int = 108;
pub const AF_ECONET: c_int = 109;
pub const AF_IEEE802154: c_int = 110;
pub const AF_IPX: c_int = 111;
pub const AF_IRDA: c_int = 112;
pub const AF_ISDN: c_int = 113;
pub const AF_IUCV: c_int = 114;
pub const AF_KEY: c_int = 115;
pub const AF_LLC: c_int = 116;
pub const AF_NETBEUI: c_int = 117;
pub const AF_NETLINK: c_int = 118;
pub const AF_NETROM: c_int = 119;
pub const AF_PACKET: c_int = 120;
pub const AF_PHONET: c_int = 121;
pub const AF_PPPOX: c_int = 122;
pub const AF_RDS: c_int = 123;
pub const AF_ROSE: c_int = 124;
pub const AF_RXRPC: c_int = 125;
pub const AF_SECURITY: c_int = 126;
pub const AF_SNA: c_int = 127;
pub const AF_TIPC: c_int = 128;
pub const AF_WANPIPE: c_int = 129;
pub const AF_X25: c_int = 130;

// ---- *at() flags (posixc/fcntl.h) ------------------------------------------
pub const AT_FDCWD: c_int = -100;
pub const AT_EACCESS: c_int = 0x01;
pub const AT_SYMLINK_NOFOLLOW: c_int = 0x02;
pub const AT_SYMLINK_FOLLOW: c_int = 0x04;
pub const AT_REMOVEDIR: c_int = 0x08;

// ---- open()/lseek extras (posixc/fcntl.h, types/seek.h) --------------------
pub const F_OK: c_int = 0;
pub const R_OK: c_int = 4;
pub const W_OK: c_int = 2;
pub const X_OK: c_int = 1;
pub const SEEK_SET: c_int = 0;
pub const SEEK_CUR: c_int = 1;
pub const SEEK_END: c_int = 2;

// ---- stat() mode bits (POSIX-standard octal) -------------------------------
pub const S_IFMT: mode_t = 0o170000;
pub const S_IFIFO: mode_t = 0o010000;
pub const S_IFCHR: mode_t = 0o020000;
pub const S_IFDIR: mode_t = 0o040000;
pub const S_IFBLK: mode_t = 0o060000;
pub const S_IFREG: mode_t = 0o100000;
pub const S_IFLNK: mode_t = 0o120000;
pub const S_IFSOCK: mode_t = 0o140000;
pub const S_ISUID: mode_t = 0o4000;
pub const S_ISGID: mode_t = 0o2000;
pub const S_ISVTX: mode_t = 0o1000;
pub const S_IRWXU: mode_t = 0o700;
pub const S_IRUSR: mode_t = 0o400;
pub const S_IWUSR: mode_t = 0o200;
pub const S_IXUSR: mode_t = 0o100;
pub const S_IRWXG: mode_t = 0o070;
pub const S_IRGRP: mode_t = 0o040;
pub const S_IWGRP: mode_t = 0o020;
pub const S_IXGRP: mode_t = 0o010;
pub const S_IRWXO: mode_t = 0o007;
pub const S_IROTH: mode_t = 0o004;
pub const S_IWOTH: mode_t = 0o002;
pub const S_IXOTH: mode_t = 0o001;

// ---- flock() (posixc/fcntl.h) ----------------------------------------------
pub const LOCK_SH: c_int = 1;
pub const LOCK_EX: c_int = 2;
pub const LOCK_NB: c_int = 4;
pub const LOCK_UN: c_int = 8;

// ---- posix_fadvise (posixc/fcntl.h; AROS-specific ordering) ----------------
pub const POSIX_FADV_DONTNEED: c_int = 1;
pub const POSIX_FADV_NOREUSE: c_int = 2;
pub const POSIX_FADV_NORMAL: c_int = 3;
pub const POSIX_FADV_RANDOM: c_int = 4;
pub const POSIX_FADV_SEQUENTIAL: c_int = 5;
pub const POSIX_FADV_WILLNEED: c_int = 6;

// ---- fallocate flags AROS does not have (placeholders) ---------------------
pub const FALLOC_FL_KEEP_SIZE: c_int = 0x01;
pub const FALLOC_FL_PUNCH_HOLE: c_int = 0x02;
pub const FALLOC_FL_NO_HIDE_STALE: c_int = 0x04;
pub const FALLOC_FL_COLLAPSE_RANGE: c_int = 0x08;
pub const FALLOC_FL_ZERO_RANGE: c_int = 0x10;
pub const FALLOC_FL_INSERT_RANGE: c_int = 0x20;
pub const FALLOC_FL_UNSHARE_RANGE: c_int = 0x40;

// ---- statvfs flags ---------------------------------------------------------
pub const ST_RDONLY: c_ulong = 1;
pub const ST_NOSUID: c_ulong = 2;

// ---- utimensat sentinels (BSD-style) ---------------------------------------
pub const UTIME_NOW: c_long = -1;
pub const UTIME_OMIT: c_long = -2;

// ---- ioctl requests (BSD encoding) -----------------------------------------
pub const FIONBIO: c_ulong = 0x8004_667e;
pub const FIONREAD: c_ulong = 0x4004_667f;

// ---- extra socket / IP option names ----------------------------------------
pub const SCM_RIGHTS: c_int = 0x01;
pub const TCP_NODELAY: c_int = 1;
pub const TCP_MAXSEG: c_int = 2;
pub const TCP_KEEPINTVL: c_int = 512;
pub const TCP_KEEPCNT: c_int = 1024;
pub const IPV6_UNICAST_HOPS: c_int = 4;
pub const IPV6_MULTICAST_IF: c_int = 9;
pub const IPV6_MULTICAST_HOPS: c_int = 10;
pub const IPV6_MULTICAST_LOOP: c_int = 11;
pub const IPV6_V6ONLY: c_int = 26;
pub const IPV6_TCLASS: c_int = 36;
