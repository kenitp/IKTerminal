//! LAN discovery: hosts in a private IPv4 range that accept TCP on a port.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::task::JoinSet;
use tokio::time::MissedTickBehavior;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
const NAME_TIMEOUT: Duration = Duration::from_secs(3);
/// Bursts of SYNs and ARP requests can make routers, VPNs, or IDS drop other connections.
const PARALLEL: usize = 64;
/// At most 200 probes per second.
const PROBE_INTERVAL: Duration = Duration::from_millis(5);
const MIN_PREFIX: u8 = 16;
const MAX_PREFIX: u8 = 30;
const PRIVATE: [(Ipv4Addr, u8); 3] = [
    (Ipv4Addr::new(10, 0, 0, 0), 8),
    (Ipv4Addr::new(172, 16, 0, 0), 12),
    (Ipv4Addr::new(192, 168, 0, 0), 16),
];

/// A private IPv4 network from /16 to /30.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    network: Ipv4Addr,
    prefix: u8,
}

impl Range {
    /// The network of `prefix` that holds `addr`.
    pub fn around(addr: Ipv4Addr, prefix: u8) -> Result<Self, String> {
        if !(MIN_PREFIX..=MAX_PREFIX).contains(&prefix) {
            return Err(format!(
                "/{MIN_PREFIX} から /{MAX_PREFIX} で指定してください"
            ));
        }
        let range = Self {
            network: Ipv4Addr::from(u32::from(addr) & mask(prefix)),
            prefix,
        };
        if !PRIVATE.iter().any(|&(net, len)| {
            len <= prefix && u32::from(range.network) & mask(len) == u32::from(net)
        }) {
            return Err(format!(
                "{range} はプライベートアドレス (10/8, 172.16/12, 192.168/16) ではありません"
            ));
        }
        Ok(range)
    }

    /// Choices for an interface: its own network when it is /24 to /30, otherwise its /24
    /// and its network widened to at least /16. Point-to-point and public addresses give none.
    pub fn for_interface(addr: Ipv4Addr, prefix: u8) -> Vec<Self> {
        let prefixes: &[u8] = match prefix {
            31.. => &[],
            24..=30 => &[prefix],
            _ => &[24, prefix.max(MIN_PREFIX)],
        };
        prefixes
            .iter()
            .filter_map(|&prefix| Self::around(addr, prefix).ok())
            .collect()
    }

    /// `a.b.c.d/n`. Host bits are cleared.
    pub fn parse(text: &str) -> Result<Self, String> {
        let invalid = || "192.168.1.0/24 の形で入力してください".to_owned();
        let (addr, prefix) = text.trim().split_once('/').ok_or_else(invalid)?;
        let addr: Ipv4Addr = addr.trim().parse().map_err(|_| invalid())?;
        let prefix: u8 = prefix.trim().parse().map_err(|_| invalid())?;
        Self::around(addr, prefix)
    }

    fn size(self) -> u32 {
        1 << (32 - self.prefix)
    }

    pub fn contains(self, addr: Ipv4Addr) -> bool {
        u32::from(addr) & mask(self.prefix) == u32::from(self.network)
    }

    /// Hosts probed when `local` is this machine's address.
    pub fn count(self, local: Option<Ipv4Addr>) -> u32 {
        self.size() - 2 - u32::from(local.is_some_and(|local| self.is_host(local)))
    }

    fn is_host(self, addr: Ipv4Addr) -> bool {
        let base = u32::from(self.network);
        let addr32 = u32::from(addr);
        self.contains(addr) && addr32 != base && addr32 != base + self.size() - 1
    }

    /// Hosts without the network and broadcast addresses and `local`. The /24 that holds
    /// `local` comes first, then its neighbors.
    fn hosts(self, local: Option<Ipv4Addr>) -> impl Iterator<Item = Ipv4Addr> {
        let base = u32::from(self.network);
        let block_len = self.size().min(256);
        let near = local
            .filter(|&local| self.contains(local))
            .map_or(0, |local| (u32::from(local) - base) / 256);
        let mut blocks: Vec<u32> = (0..self.size() / block_len).collect();
        blocks.sort_by_key(|block| block.abs_diff(near));
        blocks
            .into_iter()
            .flat_map(move |block| (0..block_len).map(move |i| base + block * 256 + i))
            .map(Ipv4Addr::from)
            .filter(move |&addr| self.is_host(addr) && Some(addr) != local)
    }
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.network, self.prefix)
    }
}

fn mask(prefix: u8) -> u32 {
    u32::MAX << (32 - u32::from(prefix))
}

#[derive(Clone, Debug)]
pub struct Device {
    pub addr: Ipv4Addr,
    pub port: u16,
    /// Reverse lookup result (DNS, mDNS, NetBIOS, ... as the OS resolver provides).
    pub name: Option<String>,
}

impl Device {
    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.addr.to_string())
    }

    /// First label of the name in lower case, or the address.
    pub fn alias(&self) -> String {
        match &self.name {
            Some(name) => name.split('.').next().unwrap_or(name).to_lowercase(),
            None => self.addr.to_string(),
        }
    }
}

enum Event {
    Found(Device),
    Named(Ipv4Addr, String),
    Done,
}

#[derive(Default)]
struct Progress {
    /// Address most recently dispatched, as `u32`.
    current: AtomicU32,
    done: AtomicU32,
}

/// Scan results collected for the sidebar.
#[derive(Default)]
pub struct Scanner {
    /// Kept after the probes finish until the last name lookup ends.
    rx: Option<Receiver<Event>>,
    scanning: bool,
    progress: Arc<Progress>,
    total: u32,
    pub devices: Vec<Device>,
    pub message: Option<String>,
}

impl Scanner {
    pub fn scanning(&self) -> bool {
        self.scanning
    }

    /// Address being probed, probes finished, and probes in total.
    pub fn progress(&self) -> Option<(Ipv4Addr, u32, u32)> {
        self.scanning().then(|| {
            (
                Ipv4Addr::from(self.progress.current.load(Ordering::Relaxed)),
                self.progress.done.load(Ordering::Relaxed),
                self.total,
            )
        })
    }

    /// With `names`, devices are reverse-resolved after they are found.
    pub fn start(
        &mut self,
        range: Range,
        port: u16,
        names: bool,
        wake: impl Fn() + Send + Sync + 'static,
    ) {
        self.devices.clear();
        self.message = None;
        let local = own_address(range);
        self.total = range.count(local);
        self.progress = Arc::default();
        self.scanning = true;
        self.rx = Some(spawn(
            range.hosts(local),
            port,
            names,
            self.progress.clone(),
            Arc::new(wake),
        ));
    }

    /// Takes new results. Devices for which `keep` is false are dropped.
    pub fn poll(&mut self, keep: impl Fn(&Device) -> bool) {
        let Some(rx) = &self.rx else { return };
        loop {
            match rx.try_recv() {
                Ok(Event::Found(device)) => {
                    if keep(&device) {
                        let at = self.devices.partition_point(|d| d.addr < device.addr);
                        self.devices.insert(at, device);
                    }
                }
                Ok(Event::Named(addr, name)) => {
                    if let Some(at) = self.devices.iter().position(|d| d.addr == addr) {
                        self.devices[at].name = Some(name);
                        if !keep(&self.devices[at]) {
                            self.devices.remove(at);
                        }
                    }
                }
                Ok(Event::Done) => {
                    self.scanning = false;
                    if self.devices.is_empty() {
                        self.message = Some("未登録の機器は見つかりません".to_owned());
                    }
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.rx = None;
                    return;
                }
            }
        }
    }

    pub fn retain(&mut self, keep: impl Fn(&Device) -> bool) {
        self.devices.retain(keep);
    }
}

fn spawn(
    mut addrs: impl Iterator<Item = Ipv4Addr> + Send + 'static,
    port: u16,
    names: bool,
    progress: Arc<Progress>,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> Receiver<Event> {
    let (tx, rx) = mpsc::channel();
    super::runtime().spawn(async move {
        let mut tasks = JoinSet::new();
        let mut pace = tokio::time::interval(PROBE_INTERVAL);
        pace.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            while tasks.len() < PARALLEL
                && let Some(addr) = addrs.next()
            {
                pace.tick().await;
                progress.current.store(u32::from(addr), Ordering::Relaxed);
                let tx = tx.clone();
                let wake = wake.clone();
                let progress = progress.clone();
                tasks.spawn(async move {
                    let open = tokio::time::timeout(
                        CONNECT_TIMEOUT,
                        TcpStream::connect(SocketAddr::from((addr, port))),
                    )
                    .await
                    .is_ok_and(|r| r.is_ok());
                    progress.done.fetch_add(1, Ordering::Relaxed);
                    if open {
                        let _ = tx.send(Event::Found(Device {
                            addr,
                            port,
                            name: None,
                        }));
                        wake();
                        if names {
                            tokio::spawn(lookup(tx, wake, addr));
                        }
                    }
                });
            }
            if tasks.join_next().await.is_none() {
                break;
            }
        }
        let _ = tx.send(Event::Done);
        wake();
    });
    rx
}

/// The OS resolver may wait on NetBIOS or LLMNR for a long time, so the name is optional.
async fn lookup(tx: Sender<Event>, wake: Arc<dyn Fn() + Send + Sync>, addr: Ipv4Addr) {
    let resolve = tokio::task::spawn_blocking(move || reverse_name(addr));
    if let Ok(Ok(Some(name))) = tokio::time::timeout(NAME_TIMEOUT, resolve).await {
        let _ = tx.send(Event::Named(addr, name));
        wake();
    }
}

/// This machine's address inside `range`, if any interface has one.
fn own_address(range: Range) -> Option<Ipv4Addr> {
    super::netif::interfaces()
        .into_iter()
        .map(|interface| interface.addr)
        .find(|&addr| range.contains(addr))
}

/// The address of the interface that holds the default route. No packet is sent.
pub fn local_ipv4() -> std::io::Result<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9))?;
    match socket.local_addr()?.ip() {
        IpAddr::V4(ip) if !ip.is_unspecified() && !ip.is_loopback() => Ok(ip),
        _ => Err(std::io::Error::other("IPv4 アドレスがありません")),
    }
}

#[cfg(any(windows, target_os = "linux"))]
#[repr(C)]
struct SockaddrIn {
    family: u16,
    port: u16,
    addr: [u8; 4],
    zero: [u8; 8],
}

#[cfg(any(windows, target_os = "linux"))]
fn sockaddr(addr: Ipv4Addr) -> SockaddrIn {
    const AF_INET: u16 = 2;
    SockaddrIn {
        family: AF_INET,
        port: 0,
        addr: addr.octets(),
        zero: [0; 8],
    }
}

/// Winsock is already started: `local_ipv4` opened a std socket before any lookup.
#[cfg(windows)]
fn reverse_name(addr: Ipv4Addr) -> Option<String> {
    const NI_NAMEREQD: i32 = 4;
    let sa = sockaddr(addr);
    let mut host = [0u16; 1025];
    let code = unsafe {
        GetNameInfoW(
            &sa,
            size_of::<SockaddrIn>() as i32,
            host.as_mut_ptr(),
            host.len() as u32,
            std::ptr::null_mut(),
            0,
            NI_NAMEREQD,
        )
    };
    let len = host.iter().position(|&c| c == 0)?;
    (code == 0 && len > 0).then(|| String::from_utf16_lossy(&host[..len]))
}

#[cfg(windows)]
#[link(name = "ws2_32")]
unsafe extern "system" {
    fn GetNameInfoW(
        addr: *const SockaddrIn,
        addr_len: i32,
        host: *mut u16,
        host_len: u32,
        service: *mut u16,
        service_len: u32,
        flags: i32,
    ) -> i32;
}

#[cfg(target_os = "linux")]
fn reverse_name(addr: Ipv4Addr) -> Option<String> {
    const NI_NAMEREQD: i32 = 8;
    let sa = sockaddr(addr);
    let mut host = [0u8; 1025];
    let code = unsafe {
        getnameinfo(
            &sa,
            size_of::<SockaddrIn>() as u32,
            host.as_mut_ptr(),
            host.len() as u32,
            std::ptr::null_mut(),
            0,
            NI_NAMEREQD,
        )
    };
    let len = host.iter().position(|&c| c == 0)?;
    (code == 0 && len > 0).then(|| String::from_utf8_lossy(&host[..len]).into_owned())
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn getnameinfo(
        addr: *const SockaddrIn,
        addr_len: u32,
        host: *mut u8,
        host_len: u32,
        service: *mut u8,
        service_len: u32,
        flags: i32,
    ) -> i32;
}

#[cfg(not(any(windows, target_os = "linux")))]
fn reverse_name(_addr: Ipv4Addr) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_skip_network_broadcast_and_local() {
        let local = Ipv4Addr::new(192, 168, 1, 10);
        let range = Range::around(local, 24).unwrap();
        let all: Vec<_> = range.hosts(Some(local)).collect();
        assert_eq!(all.len(), 253);
        assert_eq!(range.count(Some(local)), 253);
        assert_eq!(all[0], Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(*all.last().unwrap(), Ipv4Addr::new(192, 168, 1, 254));
        assert!(!all.contains(&local));

        let local = Ipv4Addr::new(172, 24, 163, 106);
        let range = Range::around(local, 16).unwrap();
        let all: Vec<_> = range.hosts(Some(local)).collect();
        assert_eq!(all.len(), (1 << 16) - 3);
        assert_eq!(range.count(Some(local)), (1 << 16) - 3);
        assert_eq!(all[0], Ipv4Addr::new(172, 24, 163, 0));
        assert_eq!(all[255], Ipv4Addr::new(172, 24, 162, 0));
        assert!(!all.contains(&local));
        assert!(!all.contains(&Ipv4Addr::new(172, 24, 0, 0)));
        assert!(!all.contains(&Ipv4Addr::new(172, 24, 255, 255)));
        assert!(all.contains(&Ipv4Addr::new(172, 24, 255, 254)));

        let other = Range::parse("10.0.5.0/30").unwrap();
        assert_eq!(other.count(Some(local)), 2);
        let all: Vec<_> = other.hosts(Some(local)).collect();
        assert_eq!(
            all,
            [Ipv4Addr::new(10, 0, 5, 1), Ipv4Addr::new(10, 0, 5, 2)]
        );
    }

    #[test]
    fn interface_choices_follow_its_prefix() {
        let show = |addr: [u8; 4], prefix| -> Vec<String> {
            Range::for_interface(Ipv4Addr::from(addr), prefix)
                .iter()
                .map(Range::to_string)
                .collect()
        };
        assert_eq!(show([192, 168, 1, 100], 24), ["192.168.1.0/24"]);
        assert_eq!(show([192, 168, 1, 100], 28), ["192.168.1.96/28"]);
        assert_eq!(
            show([172, 27, 48, 1], 20),
            ["172.27.48.0/24", "172.27.48.0/20"]
        );
        assert_eq!(show([10, 1, 2, 3], 8), ["10.1.2.0/24", "10.1.0.0/16"]);
        assert!(show([10, 10, 10, 117], 32).is_empty());
        assert!(show([100, 81, 214, 2], 24).is_empty());
    }

    #[test]
    fn range_accepts_only_private_networks() {
        let range = Range::parse(" 192.168.1.77 / 24 ").unwrap();
        assert_eq!(range.to_string(), "192.168.1.0/24");
        assert_eq!(
            Range::parse("172.31.0.0/16").unwrap().to_string(),
            "172.31.0.0/16"
        );
        assert!(Range::parse("172.32.0.0/16").is_err());
        assert!(Range::parse("8.8.8.0/24").is_err());
        assert!(Range::parse("10.0.0.0/8").is_err());
        assert!(Range::parse("192.168.1.0/31").is_err());
        assert!(Range::parse("192.168.1.0").is_err());
        assert!(Range::parse("192.168.1.0/x").is_err());
    }

    #[test]
    fn alias_uses_the_first_label() {
        let device = Device {
            addr: Ipv4Addr::new(10, 0, 0, 5),
            port: 22,
            name: Some("RaspberryPi.lan".to_owned()),
        };
        assert_eq!(device.alias(), "raspberrypi");
        let bare = Device {
            name: None,
            ..device
        };
        assert_eq!(bare.alias(), "10.0.0.5");
    }
}
