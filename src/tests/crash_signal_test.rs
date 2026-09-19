//! Crash-signal record formatting (#352).
//!
//! `opencrabs-ops` was killed by SIGBUS and logged nothing, because the daemon
//! installs no signal handler: a fatal fault signal goes to the kernel's
//! default disposition and the process dies silently. The fix records the
//! decisive facts before the process dies — signal, `si_code`, `si_addr`,
//! faulting instruction pointer, pid/tid.
//!
//! These tests pin the **pure formatter core**, which is why it exists as a
//! separate layer from the handler: a signal handler may not allocate or lock,
//! so it cannot be exercised by ordinary tests. The handler that *fills* these
//! structures is covered by the re-exec integration test below it.

use crate::logging::crash::{
    BACKTRACE_BUF_LEN, CrashFacts, HEADER_BUF_LEN, MAX_FRAMES, render_backtrace, render_header,
    si_code_name, signal_name,
};

/// A truncated-mmap SIGBUS — the memory-pressure shape from #352.
fn bus_objerr() -> CrashFacts {
    CrashFacts {
        signo: 7,
        si_code: 3,
        si_addr: 0x7ffe1234,
        rip: 0x55a1b2c3,
        pid: 1234,
        tid: 1235,
        base: 0x55a10000,
    }
}

/// A null-pointer SIGSEGV, for contrast.
fn segv_misaligned() -> CrashFacts {
    CrashFacts {
        signo: 11,
        si_code: 1,
        si_addr: 0x0,
        rip: 0xdeadbeef,
        pid: 99,
        tid: 100,
        base: 0x55a10000,
    }
}

fn render_to_string(facts: &CrashFacts) -> String {
    let mut buf = [0u8; HEADER_BUF_LEN];
    let n = render_header(facts, &mut buf);
    String::from_utf8(buf[..n].to_vec()).expect("header record is valid UTF-8")
}

#[test]
fn header_record_for_bus_objerr_is_byte_exact() {
    assert_eq!(
        render_to_string(&bus_objerr()),
        "CRASH signo=7(SIGBUS) si_code=3(BUS_OBJERR) si_addr=0x7ffe1234 \
         rip=0x55a1b2c3 base=0x55a10000 pid=1234 tid=1235\n"
    );
}

#[test]
fn header_record_for_segv_is_byte_exact() {
    assert_eq!(
        render_to_string(&segv_misaligned()),
        "CRASH signo=11(SIGSEGV) si_code=1(SEGV_MAPERR) si_addr=0x0 \
         rip=0xdeadbeef base=0x55a10000 pid=99 tid=100\n"
    );
}

/// The whole point of the decoder: `si_code` is only meaningful *relative to
/// its signal*. Code `1` names four different faults under four signals, and a
/// signal-blind table would report the same name for all of them.
#[test]
fn si_code_decoding_is_signal_specific() {
    assert_eq!(si_code_name(7, 1), "BUS_ADRALN");
    assert_eq!(si_code_name(11, 1), "SEGV_MAPERR");
    assert_eq!(si_code_name(4, 1), "ILL_ILLOPC");
    assert_eq!(si_code_name(8, 1), "FPE_INTDIV");
}

/// The decisive discrimination for #352: `BUS_OBJERR`/`BUS_ADRERR` are the
/// truncated-mmap shapes, `BUS_MCEERR_*` is a hardware machine check, and
/// `BUS_ADRALN` is a misaligned access. They must never collapse into one name.
#[test]
fn sigbus_fault_classes_stay_distinct() {
    assert_eq!(si_code_name(7, 2), "BUS_ADRERR");
    assert_eq!(si_code_name(7, 3), "BUS_OBJERR");
    assert_eq!(si_code_name(7, 4), "BUS_MCEERR_AR");
    assert_eq!(si_code_name(7, 5), "BUS_MCEERR_AO");
}

/// Sender codes are shared by every signal and are signed, so the renderer
/// must not drop the sign.
#[test]
fn sender_si_codes_decode_with_their_sign() {
    assert_eq!(si_code_name(6, -6), "SI_TKILL");
    assert_eq!(si_code_name(7, 0x80), "SI_KERNEL");
    assert_eq!(si_code_name(11, 0), "SI_USER");

    let mut facts = segv_misaligned();
    facts.signo = 6;
    facts.si_code = -6;
    let record = render_to_string(&facts);
    assert!(
        record.contains("si_code=-6(SI_TKILL)"),
        "negative si_code lost its sign: {record}"
    );
}

#[test]
fn unknown_codes_degrade_to_unknown_rather_than_lying() {
    assert_eq!(si_code_name(7, 99), "UNKNOWN");
    assert_eq!(si_code_name(1234, 5678), "UNKNOWN");
    assert_eq!(signal_name(1234), "UNKNOWN");
}

#[test]
fn signal_names_cover_the_installed_set() {
    assert_eq!(signal_name(4), "SIGILL");
    assert_eq!(signal_name(6), "SIGABRT");
    assert_eq!(signal_name(7), "SIGBUS");
    assert_eq!(signal_name(8), "SIGFPE");
    assert_eq!(signal_name(11), "SIGSEGV");
}

/// The record is written from a fixed stack buffer, so a short buffer must
/// truncate rather than panic or overrun. Every prefix length is exercised, and
/// each truncated render must be a genuine prefix of the full one, filling the
/// buffer to capacity — a crash log that stops early for no reason is bytes the
/// next debugger never gets.
#[test]
fn header_render_truncates_safely_on_any_buffer_length() {
    let facts = bus_objerr();
    let mut full = [0u8; HEADER_BUF_LEN];
    let full_len = render_header(&facts, &mut full);

    for cap in 0..full_len {
        let mut small = vec![0u8; cap];
        let n = render_header(&facts, &mut small);
        assert!(n <= cap, "wrote {n} bytes into a {cap}-byte buffer");
        assert_eq!(n, cap, "truncated render left {n} of {cap} bytes unwritten");
        assert_eq!(
            &small[..n],
            &full[..n],
            "truncated render is not a prefix at cap={cap}"
        );
    }
}

/// Frames are bounded by `MAX_FRAMES` so the scratch buffer can never overflow,
/// and each frame carries both the absolute address and the base-relative
/// offset — the offset being the stable fingerprint in a stripped binary.
#[test]
fn backtrace_is_capped_and_base_relative() {
    let offsets = vec![0x55a1b2c3usize; MAX_FRAMES + 10];
    let mut buf = [0u8; BACKTRACE_BUF_LEN];
    let n = render_backtrace(0x55a10000, &offsets, &mut buf);
    let text = std::str::from_utf8(&buf[..n]).expect("backtrace block is valid UTF-8");

    assert_eq!(text.lines().count(), MAX_FRAMES);
    assert!(
        text.contains("bt[63]"),
        "last frame within the cap is missing"
    );
    assert!(
        !text.contains("bt[64]"),
        "frame beyond the cap was rendered"
    );
    assert!(
        text.starts_with("CRASH bt[0] rel=+0xb2c3 abs=0x55a1b2c3\n"),
        "first frame is not base-relative: {text}"
    );
}

/// A frame below the executable base (a libc frame) must render as a negative
/// offset, not as a huge positive one.
#[test]
fn frames_below_base_render_negative() {
    let offsets = [0x55a10000usize, 0x55a0f000];
    let mut buf = [0u8; BACKTRACE_BUF_LEN];
    let n = render_backtrace(0x55a10000, &offsets, &mut buf);
    let text = std::str::from_utf8(&buf[..n]).expect("backtrace block is valid UTF-8");

    assert_eq!(
        text,
        "CRASH bt[0] rel=+0x0 abs=0x55a10000\nCRASH bt[1] rel=-0x1000 abs=0x55a0f000\n"
    );
}

/// No frames means no bytes — an empty block must not emit a stray newline.
#[test]
fn empty_backtrace_renders_nothing() {
    let mut buf = [0u8; BACKTRACE_BUF_LEN];
    assert_eq!(render_backtrace(0x55a10000, &[], &mut buf), 0);
}

// ---------------------------------------------------------------------------
// Re-exec integration test — the handler, on a real process
// ---------------------------------------------------------------------------
//
// The handler cannot be exercised in-process: it terminates the process by
// design. So the parent test re-executes this same test binary with a filter
// that selects one `#[ignore]`d probe, and that child installs the handler and
// faults for real.
//
// Two properties are asserted, and the first is the whole point of the feature:
// the child dies by the TRUE signal rather than a clean exit (a handler that
// swallowed the fault would exit 0), and the decisive record reached both
// `crash.log` and stderr.
//
// Gated to x86_64 Linux because that is the only target with a real handler —
// elsewhere `install_crash_handler` returns `ErrorKind::Unsupported`, so the
// probe would panic and exit cleanly instead of dying by signal.

/// Env marker set on the re-executed child, so the probe knows it is the child.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const PROBE_ENV: &str = "OC_CRASH_PROBE_CHILD";

/// Libtest filter that selects the probe alone in the child process.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const PROBE_TEST: &str = "crash_probe_child";

/// Touch a page of a mapping whose backing file has been truncated to zero.
///
/// This is the #352 shape: `SIGBUS`/`BUS_ADRERR` carrying a real `si_addr`,
/// rather than a raised signal that leaves sender credentials in the union.
/// Returns `false` when the mapping cannot be set up, so the caller can fall
/// back to a raised `SIGBUS` instead of passing silently.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn fault_via_truncated_mmap(dir: &std::path::Path) -> bool {
    use std::os::fd::AsRawFd;

    let path = dir.join("truncated-mmap.bin");
    let Ok(file) = std::fs::File::create(&path) else {
        return false;
    };
    if file.set_len(4096).is_err() {
        return false;
    }

    let ptr = unsafe {
        libc::mmap(
            core::ptr::null_mut(),
            4096,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            file.as_raw_fd(),
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return false;
    }
    if file.set_len(0).is_err() {
        return false;
    }

    // Reading past the now zero-length backing file faults. `read_volatile`
    // cannot be optimised away, unlike a plain read whose value is unused.
    let _ = unsafe { core::ptr::read_volatile(ptr.cast::<u8>()) };
    true
}

/// The child half: install the handler, then fault.
///
/// Ignored by default because it terminates the process by signal. It acts only
/// when the parent re-execs the test binary with [`PROBE_ENV`] set.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
#[ignore = "terminates the process by signal; run as a child by the re-exec test"]
fn crash_probe_child() {
    if std::env::var_os(PROBE_ENV).is_none() {
        return;
    }

    crate::logging::install_crash_handler().expect("crash handler installs");

    let dir = std::env::var_os("DEBUG_LOGS_LOCATION")
        .map(std::path::PathBuf::from)
        .expect("the parent points DEBUG_LOGS_LOCATION at a temp dir");

    if !fault_via_truncated_mmap(&dir) {
        // No mapping: a raised SIGBUS still exercises the handler, its record
        // and the re-raise. Only the fault address is lost.
        unsafe { libc::raise(libc::SIGBUS) };
    }

    // Reachable only if the handler returned instead of dying — the exact
    // failure this feature exists to prevent. A clean exit fails the parent.
    std::process::exit(0);
}

/// The parent half: re-exec the probe and assert on how it died.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn child_crash_records_and_dies_by_true_signal() {
    use std::os::unix::process::ExitStatusExt;

    if std::env::var_os(PROBE_ENV).is_some() {
        // We are the child, running the whole binary under a probe filter.
        return;
    }

    let exe = std::env::current_exe().expect("test binary path");
    let home = tempfile::tempdir().expect("temp dir");

    let output = std::process::Command::new(exe)
        .arg(PROBE_TEST)
        .args(["--ignored", "--nocapture", "--test-threads=1"])
        .env(PROBE_ENV, "1")
        .env("DEBUG_LOGS_LOCATION", home.path())
        .output()
        .expect("child test binary runs");

    let stderr = String::from_utf8_lossy(&output.stderr);

    // (a) Anti-masking. A clean exit would mean the handler swallowed the fault
    // — the one thing it must never do. A zero-test child also lands here.
    assert_eq!(
        output.status.signal(),
        Some(libc::SIGBUS),
        "child must die by SIGBUS, not exit cleanly (status={:?})\n--- child stderr ---\n{stderr}",
        output.status
    );

    // (b) The decisive record reached the crash log beside the temp log dir.
    let log_path = home.path().join("crash.log");
    let log = std::fs::read_to_string(&log_path).unwrap_or_else(|e| {
        panic!(
            "crash.log missing at {}: {e}\n--- child stderr ---\n{stderr}",
            log_path.display()
        )
    });
    assert!(
        log.contains("CRASH signo=7(SIGBUS)"),
        "crash.log lacks the decisive header:\n{log}"
    );

    // (c) The same record reached stderr, so a host with an unwritable log dir
    // still sees the fault in whatever captured the daemon's stderr.
    assert!(
        stderr.contains("CRASH signo=7(SIGBUS)"),
        "child stderr lacks the decisive header:\n{stderr}"
    );
}
