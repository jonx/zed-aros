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
pub type time_t = i64;
pub type suseconds_t = i64;

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
}

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
