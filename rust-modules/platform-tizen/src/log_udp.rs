//! The log channel: every event-log line, as one UDP datagram, to an address the developer configured.
//!
//! A Tizen app cannot be read off the device (no `sdb shell`, no file pull: `docs/platform-ports.md`, E5), so
//! the event log leaves the process over the network. [`install`] makes this crate's sender the
//! platform's tap on the event log (`plx_base::eventlog::set_tap`): the log hands the tap each SCRUBBED
//! line, and only the lines the network exit allows (a line that still looks like a credential is kept in
//! the file and never offered here).
//!
//! The channel is **off unless the launch string names an IPv4 literal target** (`Launch::log_udp`): no
//! name lookup (a blocking `getaddrinfo` has no place in a logger), no default address. A datagram is
//! `"<seq:06> <line>\n"`: the sequence number is per sender and advances on every line whether or not the
//! send worked, so a lost datagram is a gap at the receiver. The socket is non-blocking and a failed send
//! is dropped: the tap runs on whichever thread logged, the frame loop included, and must never wait.
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// Largest datagram: under a 1500-byte Ethernet MTU less the IP and UDP headers. A longer line is cut
/// (marked `...`) in the datagram only; the file keeps it whole.
const DATAGRAM_LIMIT: usize = 1400;

/// One UDP sender: a non-blocking socket, its target and its sequence counter.
pub struct UdpLog {
    socket: UdpSocket,
    target: SocketAddrV4,
    seq: AtomicU64,
}

impl UdpLog {
    /// Bind an ephemeral local port and switch the socket to non-blocking.
    pub fn open(target: SocketAddrV4) -> io::Result<UdpLog> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        socket.set_nonblocking(true)?;
        Ok(UdpLog { socket, target, seq: AtomicU64::new(0) })
    }

    /// Send `line` as one datagram. The sequence advances whether or not the send worked; a failure is
    /// ignored, because there is nowhere to report it that is not this channel.
    pub fn send_line(&self, line: &str) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let _ = self.socket.send_to(datagram(seq, line).as_bytes(), self.target);
    }
}

/// `"<seq:06> <line>\n"`, the line's own line breaks flattened to spaces, cut to [`DATAGRAM_LIMIT`] bytes on
/// a character boundary and ended `...\n` when it was cut.
fn datagram(seq: u64, line: &str) -> String {
    let flat: String = line.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
    let mut s = format!("{seq:06} {flat}\n");
    if s.len() > DATAGRAM_LIMIT {
        let mut cut = DATAGRAM_LIMIT - 4;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push_str("...\n");
    }
    s
}

/// The process's one sender, once installed. The event log's tap is a plain `fn(&str)`, so what it sends
/// through lives here.
static SENDER: OnceLock<UdpLog> = OnceLock::new();

fn tap(line: &str) {
    if let Some(sender) = SENDER.get() {
        sender.send_line(line);
    }
}

/// Why [`install`] left the channel off.
#[derive(Debug)]
pub enum InstallError {
    /// The socket could not be made.
    Socket(io::Error),
    /// A channel was installed already (once per process).
    Installed,
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Socket(e) => write!(f, "the socket could not be opened ({e})"),
            InstallError::Installed => f.write_str("a log channel is already installed"),
        }
    }
}

/// Turn the channel on: open the socket to `target` and make it the event log's tap. Once per process.
/// Called on the host thread inside `plx_host_start`, before the application's loop exists, so the first
/// line the loop logs is already on the wire.
pub fn install(target: SocketAddrV4) -> Result<(), InstallError> {
    let sender = UdpLog::open(target).map_err(InstallError::Socket)?;
    SENDER.set(sender).map_err(|_| InstallError::Installed)?;
    // The tap is set last: it reads `SENDER`, which is set by now. A second `set_tap` is refused, which
    // only happens when something else in the process claimed the tap first.
    plx_base::eventlog::set_tap(tap).map_err(|_| InstallError::Installed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn receiver() -> (UdpSocket, SocketAddrV4) {
        let rx = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind a receiver");
        rx.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let std::net::SocketAddr::V4(addr) = rx.local_addr().unwrap() else { panic!("not IPv4") };
        (rx, addr)
    }

    fn recv(rx: &UdpSocket) -> String {
        let mut buf = [0u8; 2048];
        let n = rx.recv(&mut buf).expect("a datagram within 2 s");
        String::from_utf8(buf[..n].to_vec()).expect("UTF-8")
    }

    /// One datagram per line, in order, each with its sequence number.
    #[test]
    fn datagrams_arrive_in_order_with_a_sequence_prefix() {
        let (rx, addr) = receiver();
        let tx = UdpLog::open(addr).unwrap();
        tx.send_line("first");
        tx.send_line("second\nstill second");
        assert_eq!(recv(&rx), "000000 first\n");
        assert_eq!(recv(&rx), "000001 second still second\n", "a line break inside a line is flattened");
    }

    /// A send that fails is dropped and the sequence still advances, so the receiver sees a gap and the
    /// sender never reports, retries or waits.
    #[test]
    fn a_failed_send_leaves_a_gap_in_the_sequence() {
        let (rx, addr) = receiver();
        let mut tx = UdpLog::open(addr).unwrap();
        tx.target = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0); // port 0 is no destination
        tx.send_line("lost");
        tx.target = addr;
        tx.send_line("kept");
        assert_eq!(recv(&rx), "000001 kept\n");
    }

    /// **The socket is non-blocking**, asserted directly: a read on the sender's own socket, which nothing
    /// ever writes to, returns `WouldBlock` at once instead of waiting. (A send to a full loopback receiver is
    /// dropped by the kernel whatever the mode, so a burst cannot tell the two apart.) A burst far larger than
    /// any socket buffer must also return and still number every line.
    #[test]
    fn a_send_never_waits_for_a_receiver_that_does_not_read() {
        let (_rx, addr) = receiver();
        let tx = UdpLog::open(addr).unwrap();
        let before = std::time::Instant::now();
        let err = tx.socket.recv(&mut [0u8; 16]).expect_err("nothing was sent to the sender's socket");
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock, "a blocking socket would have waited here");
        assert!(before.elapsed() < Duration::from_secs(1), "returned at once");
        let started = std::time::Instant::now();
        for i in 0..20_000 {
            tx.send_line(&format!("burst {i} {}", "x".repeat(900)));
        }
        assert!(started.elapsed() < Duration::from_secs(10), "20000 sends took {:?}", started.elapsed());
        assert_eq!(tx.seq.load(Ordering::Relaxed), 20_000, "every line took a sequence number");
    }

    #[test]
    fn a_long_line_is_cut_on_a_character_boundary_and_stays_one_datagram() {
        let long = "é".repeat(1000);
        let d = datagram(7, &long);
        assert!(d.len() <= DATAGRAM_LIMIT, "{} bytes", d.len());
        assert!(d.starts_with("000007 é") && d.ends_with("...\n"));
        assert_eq!(datagram(8, "short"), "000008 short\n");
    }

    /// **Off unless configured**: nothing is sent, and the tap is not even set, until [`install`] runs. This
    /// is the one test that installs the process-wide tap (it is once per process), and it proves each claim
    /// in order: before the install a logged line reaches nobody; after it the receiver gets the scrubbed
    /// line behind a sequence number; a second install is refused.
    #[test]
    fn the_channel_is_off_until_installed_and_then_carries_the_log() {
        let (rx, addr) = receiver();
        plx_base::eventlog::with_private_log(|| {
            // Before the install there is no sender for the tap to send through: the channel is off, whatever
            // the receiver would have heard. (This test is the only one that installs.)
            assert!(SENDER.get().is_none(), "no sender exists before the install");
            plx_base::eventlog::log("udp-test: before the install");
            tap("udp-test: a tap call with no sender sends nothing, and does not fail");
            install(addr).expect("the first install is accepted");
            plx_base::eventlog::log("udp-test: after the install X-Plex-Token=SECRETQQ99 ok");
            // Other tests in this process log on their own threads and the tap is process-wide, so read
            // until this test's own line arrives.
            assert!(SENDER.get().is_some(), "the install made the sender");
            let got = (0..200)
                .map(|_| recv(&rx))
                .inspect(|d| assert!(!d.contains("udp-test: before the install"), "an off channel sent {d:?}"))
                .find(|d| d.contains("udp-test: after the install"))
                .expect("this test's line reaches the receiver");
            let (seq, _) = got.split_once(' ').expect("a sequence prefix");
            assert!(seq.len() == 6 && seq.bytes().all(|b| b.is_ascii_digit()), "{got:?}");
            assert!(got.contains("X-Plex-Token=<redacted>") && !got.contains("SECRETQQ99"), "scrubbed: {got:?}");
            assert!(matches!(install(addr), Err(InstallError::Installed)), "a second install is refused");
        });
    }
}
