//! Crash-signal diagnostics — pure record formatting (#352).
//!
//! `opencrabs-ops` was killed by SIGBUS under host memory pressure and logged
//! **nothing**: the daemon installs no signal handler at all, so a fatal fault
//! signal is delivered to the kernel's default disposition and the process dies
//! silently. Only the supervisor records a death, and it records neither the
//! signal nor the faulting address — the mechanism is therefore unfalsifiable.
//!
//! This module holds the **pure core** of the diagnostic: the facts captured at
//! fault time and the renderers that turn them into bytes. It allocates
//! nothing, locks nothing, and contains no `unsafe` — which is precisely what
//! makes it unit-testable. The handler that fills these structures — and the
//! installer that registers it — live in this module, gated to x86_64 Linux.
//!
//! # Record shape
//!
//! The header is the decisive record and is always written first:
//!
//! ```text
//! CRASH signo=7(SIGBUS) si_code=3(BUS_OBJERR) si_addr=0x7ffe1234 rip=0x55a1b2c3 base=0x55a10000 pid=1234 tid=1235
//! ```
//!
//! Backtrace frames follow, best-effort:
//!
//! ```text
//! CRASH bt[0] rel=+0x1a2b3c abs=0x55a1b2c3d4e5
//! ```
//!
//! `si_code` is the field that decides *which* fault this was: for SIGBUS it
//! separates `BUS_ADRERR`/`BUS_OBJERR` (truncated or missing mmap backing — the
//! memory-pressure hypothesis) from `BUS_MCEERR_*` (hardware machine check) and
//! `BUS_ADRALN` (misaligned access). Values are cited from
//! `/usr/include/asm-generic/siginfo.h`; they are the generic set, which is what
//! x86_64 uses.

/// Maximum number of backtrace frames recorded.
pub const MAX_FRAMES: usize = 64;

/// Scratch buffer size for the header record (one line, always written first).
pub const HEADER_BUF_LEN: usize = 512;

/// Scratch buffer size for the backtrace block.
pub const BACKTRACE_BUF_LEN: usize = 4096;

/// Facts captured inside the signal handler.
///
/// Every field is plain data: no `String`, no `Vec`, nothing that allocates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrashFacts {
    /// Signal number (`si_signo`).
    pub signo: i32,
    /// Signal-specific fault code (`si_code`).
    pub si_code: i32,
    /// Faulting address (`si_addr`), meaningful for SIGBUS/SIGSEGV.
    pub si_addr: usize,
    /// Faulting instruction pointer, from the signal context.
    pub rip: usize,
    /// Process id.
    pub pid: i32,
    /// Thread id of the faulting thread.
    pub tid: i32,
    /// Main-executable load base, captured once at install.
    pub base: usize,
}

impl CrashFacts {
    /// A zeroed record. Useful as a test fixture and for the non-fault paths.
    pub const fn empty() -> Self {
        Self {
            signo: 0,
            si_code: 0,
            si_addr: 0,
            rip: 0,
            pid: 0,
            tid: 0,
            base: 0,
        }
    }
}

/// Human name of a signal number, or `"UNKNOWN"`.
pub fn signal_name(signo: i32) -> &'static str {
    match signo {
        4 => "SIGILL",
        6 => "SIGABRT",
        7 => "SIGBUS",
        8 => "SIGFPE",
        11 => "SIGSEGV",
        _ => "UNKNOWN",
    }
}

/// Human name of an `si_code`, decoded **per signal**.
///
/// The decoding must be signal-specific: `1` is `BUS_ADRALN` under SIGBUS,
/// `SEGV_MAPERR` under SIGSEGV, `ILL_ILLOPC` under SIGILL and `FPE_INTDIV`
/// under SIGFPE. A signal-blind table would name every one of them the same.
pub fn si_code_name(signo: i32, si_code: i32) -> &'static str {
    match signo {
        7 => match si_code {
            1 => "BUS_ADRALN",
            2 => "BUS_ADRERR",
            3 => "BUS_OBJERR",
            4 => "BUS_MCEERR_AR",
            5 => "BUS_MCEERR_AO",
            _ => generic_si_code_name(si_code),
        },
        11 => match si_code {
            1 => "SEGV_MAPERR",
            2 => "SEGV_ACCERR",
            3 => "SEGV_BNDERR",
            4 => "SEGV_PKUERR",
            5 => "SEGV_ACCADI",
            6 => "SEGV_ADIDERR",
            7 => "SEGV_ADIPERR",
            8 => "SEGV_MTEAERR",
            9 => "SEGV_MTESERR",
            10 => "SEGV_CPERR",
            _ => generic_si_code_name(si_code),
        },
        4 => match si_code {
            1 => "ILL_ILLOPC",
            2 => "ILL_ILLOPN",
            3 => "ILL_ILLADR",
            4 => "ILL_ILLTRP",
            5 => "ILL_PRVOPC",
            6 => "ILL_PRVREG",
            7 => "ILL_COPROC",
            8 => "ILL_BADSTK",
            9 => "ILL_BADIADDR",
            _ => generic_si_code_name(si_code),
        },
        8 => match si_code {
            1 => "FPE_INTDIV",
            2 => "FPE_INTOVF",
            3 => "FPE_FLTDIV",
            4 => "FPE_FLTOVF",
            5 => "FPE_FLTUND",
            6 => "FPE_FLTRES",
            7 => "FPE_FLTINV",
            8 => "FPE_FLTSUB",
            9 => "FPE_DECOVF",
            10 => "FPE_DECDIV",
            11 => "FPE_DECERR",
            12 => "FPE_INVASC",
            13 => "FPE_INVDEC",
            14 => "FPE_FLTUNK",
            15 => "FPE_CONDTRAP",
            _ => generic_si_code_name(si_code),
        },
        _ => generic_si_code_name(si_code),
    }
}

/// `si_code` values shared by every signal: who sent it, not what faulted.
fn generic_si_code_name(si_code: i32) -> &'static str {
    match si_code {
        0 => "SI_USER",
        0x80 => "SI_KERNEL",
        -1 => "SI_QUEUE",
        -2 => "SI_TIMER",
        -3 => "SI_MESGQ",
        -4 => "SI_ASYNCIO",
        -5 => "SI_SIGIO",
        -6 => "SI_TKILL",
        -7 => "SI_DETHREAD",
        -60 => "SI_ASYNCNL",
        _ => "UNKNOWN",
    }
}

/// Append-only cursor over a caller-supplied buffer.
///
/// Every push is bounds-checked and silently stops at the end of the buffer:
/// a truncated record is acceptable, a panic inside a signal handler is not.
///
/// A push that does not fit fills the remaining space and stops there, so the
/// bytes written are always a genuine prefix of the full record — never a
/// fragment of a later field grafted onto an earlier truncation point. That
/// matters when the buffer is the last thing the process will ever write: a
/// partial `si_addr=0x7f` is readable, `si_addr=0x7fpid=` is a lie.
struct Cursor<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl Cursor<'_> {
    fn new(buf: &mut [u8]) -> Cursor<'_> {
        Cursor { buf, len: 0 }
    }

    /// Copy as much of `bytes` as fits, then seal: a partial copy fills the
    /// buffer to capacity, so every later push fails its bounds check and the
    /// record ends exactly at the truncation point.
    fn push_bytes(&mut self, bytes: &[u8]) -> bool {
        let room = self.buf.len() - self.len;
        if bytes.len() > room {
            self.buf[self.len..].copy_from_slice(&bytes[..room]);
            self.len = self.buf.len();
            return false;
        }
        self.buf[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
        true
    }

    fn push_str(&mut self, s: &str) -> bool {
        self.push_bytes(s.as_bytes())
    }

    fn push_dec(&mut self, mut v: u64) -> bool {
        let mut tmp = [0u8; 20];
        let mut i = tmp.len();
        if v == 0 {
            i -= 1;
            tmp[i] = b'0';
        }
        while v > 0 {
            i -= 1;
            tmp[i] = b'0' + (v % 10) as u8;
            v /= 10;
        }
        self.push_bytes(&tmp[i..])
    }

    fn push_i64(&mut self, v: i64) -> bool {
        if v < 0 {
            if !self.push_str("-") {
                return false;
            }
            self.push_dec(v.unsigned_abs())
        } else {
            self.push_dec(v as u64)
        }
    }

    fn push_hex(&mut self, v: usize) -> bool {
        if !self.push_str("0x") {
            return false;
        }
        let mut tmp = [0u8; 16];
        let mut i = tmp.len();
        let mut v = v as u64;
        if v == 0 {
            i -= 1;
            tmp[i] = b'0';
        }
        while v > 0 {
            i -= 1;
            tmp[i] = b"0123456789abcdef"[(v & 0xf) as usize];
            v >>= 4;
        }
        self.push_bytes(&tmp[i..])
    }

    /// `signo=7(SIGBUS)` — number plus name, so the record is greppable by
    /// either and needs no lookup table to read.
    fn push_signo(&mut self, signo: i32) -> bool {
        self.push_str("signo=")
            && self.push_dec(signo.unsigned_abs() as u64)
            && self.push_str("(")
            && self.push_str(signal_name(signo))
            && self.push_str(")")
    }

    /// `si_code=3(BUS_OBJERR)` — signed, because SI_* codes are negative.
    fn push_si_code(&mut self, signo: i32, si_code: i32) -> bool {
        self.push_str("si_code=")
            && self.push_i64(si_code as i64)
            && self.push_str("(")
            && self.push_str(si_code_name(signo, si_code))
            && self.push_str(")")
    }
}

/// Render the decisive header record. Returns the number of bytes written.
///
/// This is the record the handler writes **first**, in a single `write` call,
/// before it attempts anything that could block.
pub fn render_header(facts: &CrashFacts, buf: &mut [u8]) -> usize {
    let mut c = Cursor::new(buf);
    c.push_str("CRASH ");
    c.push_signo(facts.signo);
    c.push_str(" ");
    c.push_si_code(facts.signo, facts.si_code);
    c.push_str(" si_addr=");
    c.push_hex(facts.si_addr);
    c.push_str(" rip=");
    c.push_hex(facts.rip);
    c.push_str(" base=");
    c.push_hex(facts.base);
    c.push_str(" pid=");
    c.push_dec(facts.pid.unsigned_abs() as u64);
    c.push_str(" tid=");
    c.push_dec(facts.tid.unsigned_abs() as u64);
    c.push_str("\n");
    c.len
}

/// Render the best-effort backtrace block. Returns the number of bytes written.
///
/// Each frame carries both the absolute address and the base-relative offset.
/// The offset is the crash fingerprint: the shipped binary is stripped, so
/// absolute addresses shift with ASLR but a base-relative offset is stable
/// across runs of the same build.
pub fn render_backtrace(base: usize, offsets: &[usize], buf: &mut [u8]) -> usize {
    let mut c = Cursor::new(buf);
    for (i, &addr) in offsets.iter().enumerate().take(MAX_FRAMES) {
        c.push_str("CRASH bt[");
        c.push_dec(i as u64);
        c.push_str("] rel=");
        if addr >= base {
            c.push_str("+");
            c.push_hex(addr - base);
        } else {
            c.push_str("-");
            c.push_hex(base - addr);
        }
        c.push_str(" abs=");
        c.push_hex(addr);
        c.push_str("\n");
    }
    c.len
}

// ---------------------------------------------------------------------------
// Signal handler — x86_64 Linux
// ---------------------------------------------------------------------------

/// Install the crash-signal diagnostic handler.
///
/// Registered for `SIGSEGV`, `SIGBUS`, `SIGILL`, `SIGFPE` and `SIGABRT`. On a
/// fault the handler writes one decisive line — signal, `si_code`, faulting
/// address, faulting instruction pointer, pid/tid — then a best-effort raw
/// backtrace, and finally re-raises the signal with the default disposition, so
/// the process still dies with the **true** signal and the core-dump
/// disposition is preserved. It never swallows a fault and never returns to the
/// faulting instruction.
///
/// `SIGKILL` is uncatchable and deliberately not covered: an OOM-killer kill
/// stays invisible to this handler.
///
/// Call once, after logging is initialised. `sigaltstack` is per-thread, so
/// this covers the calling thread — for the daemon, the main thread.
///
/// On any other target this returns [`std::io::ErrorKind::Unsupported`], so the
/// caller logs the absence rather than believing a handler is installed.
pub use handler::install_crash_handler;

mod handler {
    use super::{
        BACKTRACE_BUF_LEN, CrashFacts, HEADER_BUF_LEN, MAX_FRAMES, render_backtrace, render_header,
    };
    use core::ffi::{c_int, c_void};
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};

    /// Size of the dedicated signal stack. Must exceed `MINSIGSTKSZ`; 64 KiB
    /// also covers this module's scratch buffers.
    const ALT_STACK_SIZE: usize = 64 * 1024;

    /// The signals this handler is installed for.
    const FAULT_SIGNALS: [c_int; 5] = [
        libc::SIGSEGV,
        libc::SIGBUS,
        libc::SIGILL,
        libc::SIGFPE,
        libc::SIGABRT,
    ];

    /// Re-entrancy guard: a fault *inside* the handler re-raises at once rather
    /// than recursing.
    static IN_HANDLER: AtomicBool = AtomicBool::new(false);

    /// Leaked `CString` holding the crash-log path, so the handler can open it
    /// without allocating.
    static CRASH_LOG_PATH: AtomicPtr<libc::c_char> = AtomicPtr::new(core::ptr::null_mut());

    /// Main-executable load base, captured once at install.
    static EXE_BASE: AtomicUsize = AtomicUsize::new(0);

    /// Kernel-ABI mirror of `siginfo_t` for the fault shape.
    ///
    /// `libc::siginfo_t` exposes only `si_signo`/`si_errno`/`si_code`: the union
    /// that holds `si_addr` sits behind a `#[doc(hidden)]` field deprecated
    /// since 0.2.54, and CI clippy runs `-D warnings`, so that field is
    /// unusable. Stating the layout instead is sound because the kernel's
    /// `siginfo_t` is a fixed ABI — and the assertions below prove this mirror
    /// agrees with `libc::siginfo_t` wherever the two overlap, so the cast in
    /// [`facts_from`] cannot drift silently.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SigFaultInfo {
        si_signo: c_int,
        si_errno: c_int,
        si_code: c_int,
        _abi_pad: c_int,
        si_addr: *mut c_void,
    }

    const _: () = {
        assert!(
            core::mem::offset_of!(SigFaultInfo, si_signo)
                == core::mem::offset_of!(libc::siginfo_t, si_signo)
        );
        assert!(
            core::mem::offset_of!(SigFaultInfo, si_errno)
                == core::mem::offset_of!(libc::siginfo_t, si_errno)
        );
        assert!(
            core::mem::offset_of!(SigFaultInfo, si_code)
                == core::mem::offset_of!(libc::siginfo_t, si_code)
        );
        // The kernel's `siginfo_t` is 128 bytes (`SI_MAX_SIZE`); libc's struct
        // is three ints plus `_pad: [c_int; 29]`, i.e. exactly that.
        assert!(core::mem::size_of::<libc::siginfo_t>() == 128);
        // `si_addr` opens the union, 8-byte aligned after the three ints.
        assert!(core::mem::offset_of!(SigFaultInfo, si_addr) == 16);
    };

    /// Write the whole buffer, abandoning the remainder on the first error.
    ///
    /// `write` is async-signal-safe. The loop exists only because a short write
    /// is legal; `EINTR` and every other error stop the loop rather than retry,
    /// because a handler must never spin.
    unsafe fn write_all(fd: c_int, buf: &[u8]) {
        let mut rest = buf;
        while !rest.is_empty() {
            let n = unsafe { libc::write(fd, rest.as_ptr().cast::<c_void>(), rest.len()) };
            if n <= 0 {
                return;
            }
            rest = &rest[n as usize..];
        }
    }

    /// Open the crash log, or `None` when no path was registered.
    ///
    /// `open` is async-signal-safe. Opening lazily rather than at install keeps
    /// a short-lived CLI invocation from littering an empty `crash.log`.
    unsafe fn open_crash_log() -> Option<c_int> {
        let path = CRASH_LOG_PATH.load(Ordering::Acquire);
        if path.is_null() {
            return None;
        }
        let fd = unsafe {
            libc::open(
                path,
                libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND,
                0o600 as libc::mode_t,
            )
        };
        if fd < 0 { None } else { Some(fd) }
    }

    /// Capture the fault facts. Pure reads: no allocation, no locks.
    unsafe fn facts_from(signo: c_int, info: *mut libc::siginfo_t, ctx: *mut c_void) -> CrashFacts {
        let mut facts = CrashFacts::empty();
        facts.signo = signo;
        facts.pid = unsafe { libc::getpid() };
        facts.tid = unsafe { libc::syscall(libc::SYS_gettid) } as i32;
        facts.base = EXE_BASE.load(Ordering::Relaxed);

        if !info.is_null() {
            let si = info.cast::<SigFaultInfo>();
            let si_code = unsafe { (*si).si_code };
            facts.si_code = si_code;
            // A positive `si_code` means the kernel generated this from a trap,
            // and only then does the union hold a real faulting address. For
            // `SI_USER`/`SI_TKILL`/`SI_QUEUE` (<= 0) it holds sender
            // credentials instead, and printing those as an address would be a
            // lie — the address stays 0.
            if si_code > 0 {
                facts.si_addr = unsafe { (*si).si_addr } as usize;
            }
        }

        if !ctx.is_null() {
            let uc = ctx.cast::<libc::ucontext_t>();
            let rip = unsafe { (*uc).uc_mcontext.gregs[libc::REG_RIP as usize] };
            if rip > 0 {
                facts.rip = rip as usize;
            }
        }

        facts
    }

    /// Restore the default disposition, unblock, and re-raise — so the process
    /// dies with the **true** signal.
    ///
    /// The delivering signal is blocked while the handler runs (this module
    /// does not set `SA_NODEFER`), so it must be unblocked first: otherwise the
    /// re-raise would merely leave it pending and the return would re-execute
    /// the faulting instruction. `sigaction`, `sigprocmask` and `raise` are all
    /// on POSIX's async-signal-safe list.
    unsafe fn reraise(signo: c_int) -> ! {
        let mut sa: libc::sigaction = unsafe { core::mem::zeroed() };
        sa.sa_sigaction = libc::SIG_DFL;
        sa.sa_flags = 0;
        unsafe { libc::sigemptyset(&mut sa.sa_mask) };
        unsafe { libc::sigaction(signo, &sa, core::ptr::null_mut()) };

        let mut set: libc::sigset_t = unsafe { core::mem::zeroed() };
        unsafe { libc::sigemptyset(&mut set) };
        unsafe { libc::sigaddset(&mut set, signo) };
        unsafe { libc::sigprocmask(libc::SIG_UNBLOCK, &set, core::ptr::null_mut()) };

        unsafe { libc::raise(signo) };

        // Should be unreachable: with `SIG_DFL` restored, returning to the
        // faulting instruction re-faults and the kernel kills the process with
        // the true signal. This is the last-resort backstop — never a swallow.
        unsafe { libc::_exit(128 + signo) }
    }

    /// The handler itself.
    ///
    /// Ordering *is* the design: the decisive header is written with one
    /// `write` before anything that could block, so even a hang in the unwinder
    /// below leaves the record on disk.
    unsafe extern "C" fn handle(signo: c_int, info: *mut libc::siginfo_t, ctx: *mut c_void) {
        if IN_HANDLER.swap(true, Ordering::SeqCst) {
            unsafe { reraise(signo) };
        }

        let facts = unsafe { facts_from(signo, info, ctx) };

        // 1. The decisive record, written first and unconditionally.
        let mut header = [0u8; HEADER_BUF_LEN];
        let n = render_header(&facts, &mut header);
        let header = &header[..n];
        unsafe { write_all(libc::STDERR_FILENO, header) };
        let fd = unsafe { open_crash_log() };
        if let Some(fd) = fd {
            unsafe { write_all(fd, header) };
        }

        // 2. Best-effort from here: the decisive record is already written.
        let mut frames = [core::ptr::null_mut::<c_void>(); MAX_FRAMES];
        let depth = unsafe { libc::backtrace(frames.as_mut_ptr(), MAX_FRAMES as c_int) };
        if depth > 0 {
            let depth = (depth as usize).min(MAX_FRAMES);
            let mut offsets = [0usize; MAX_FRAMES];
            for (slot, frame) in offsets.iter_mut().zip(frames.iter()).take(depth) {
                *slot = *frame as usize;
            }
            let mut block = [0u8; BACKTRACE_BUF_LEN];
            let m = render_backtrace(facts.base, &offsets[..depth], &mut block);
            let block = &block[..m];
            unsafe { write_all(libc::STDERR_FILENO, block) };
            if let Some(fd) = fd {
                unsafe { write_all(fd, block) };
            }
        }

        if let Some(fd) = fd {
            unsafe { libc::close(fd) };
        }

        // 3. Die with the true signal.
        unsafe { reraise(signo) };
    }

    /// Load base of the main executable, read from `/proc/self/maps`.
    ///
    /// Called once at install (ordinary thread context, allocation allowed).
    /// Returns 0 when it cannot be determined — the record then reports
    /// `base=0x0` rather than the handler refusing to install.
    fn exe_base() -> usize {
        let Ok(exe) = std::fs::read_link("/proc/self/exe") else {
            return 0;
        };
        let exe = exe.to_string_lossy();
        let exe: &str = &exe;
        let Ok(maps) = std::fs::read_to_string("/proc/self/maps") else {
            return 0;
        };
        for line in maps.lines() {
            let mut fields = line.split_whitespace();
            let (Some(range), Some(_perms), Some(_offset), Some(_dev), Some(_inode), Some(path)) = (
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
            ) else {
                continue;
            };
            if path != exe {
                continue;
            }
            let Some((start, _end)) = range.split_once('-') else {
                continue;
            };
            if let Ok(base) = usize::from_str_radix(start, 16) {
                return base;
            }
        }
        0
    }

    /// Install the handler and its alternate stack.
    ///
    /// A failure here is reported to the caller and never aborts startup.
    pub fn install_crash_handler() -> std::io::Result<()> {
        // A dedicated signal stack, so a stack-overflow SIGSEGV is still
        // diagnosable. Leaked deliberately: it must outlive every signal.
        let mut alt_stack = vec![0u8; ALT_STACK_SIZE].into_boxed_slice();
        let ss = libc::stack_t {
            ss_sp: alt_stack.as_mut_ptr().cast::<c_void>(),
            ss_flags: 0,
            ss_size: ALT_STACK_SIZE,
        };
        core::mem::forget(alt_stack);
        if unsafe { libc::sigaltstack(&ss, core::ptr::null_mut()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }

        // Precompute the crash-log path as a NUL-terminated C string and leak
        // it: the handler may not allocate, so the path must already exist.
        //
        // `log_dir()` resolves the path but does not create it, and the handler
        // cannot create it either — `open` inside a signal context has to find
        // the directory already there. Best-effort: a failure only costs the
        // file leg of the record, never the stderr leg.
        let log_dir = crate::logging::log_dir();
        let _ = std::fs::create_dir_all(&log_dir);
        let path = log_dir.join("crash.log");
        let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "crash log path contains an interior NUL",
            )
        })?;
        CRASH_LOG_PATH.store(path.into_raw(), Ordering::Release);

        EXE_BASE.store(exe_base(), Ordering::Relaxed);

        let mut sa: libc::sigaction = unsafe { core::mem::zeroed() };
        let entry: unsafe extern "C" fn(c_int, *mut libc::siginfo_t, *mut c_void) = handle;
        sa.sa_sigaction = entry as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
        unsafe { libc::sigemptyset(&mut sa.sa_mask) };

        for signo in FAULT_SIGNALS {
            if unsafe { libc::sigaction(signo, &sa, core::ptr::null_mut()) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }

        Ok(())
    }
}
