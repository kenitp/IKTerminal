//! LAN discovery: hosts on the local 192.168 /24 or 172 /16 that accept TCP on a port.

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

    pub fn start(&mut self, port: u16, wake: impl Fn() + Send + Sync + 'static) {
        self.devices.clear();
        self.message = None;
        let local = match local_ipv4() {
            Ok(local) => local,
            Err(error) => {
                self.message = Some(format!("スキャンできません: {error}"));
                return;
            }
        };
        self.total = targets(local).count() as u32;
        if self.total == 0 {
            self.message = Some(format!("{local} はスキャン対象外です"));
            return;
        }
        self.progress = Arc::default();
        self.scanning = true;
        self.rx = Some(spawn(local, port, self.progress.clone(), Arc::new(wake)));
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
    local: Ipv4Addr,
    port: u16,
    progress: Arc<Progress>,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> Receiver<Event> {
    let (tx, rx) = mpsc::channel();
    super::runtime().spawn(async move {
        let mut addrs = targets(local);
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
                        tokio::spawn(lookup(tx, wake, addr));
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

/// The address of the interface that holds the default route. No packet is sent.
fn local_ipv4() -> std::io::Result<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9))?;
    match socket.local_addr()?.ip() {
        IpAddr::V4(ip) if !ip.is_unspecified() && !ip.is_loopback() => Ok(ip),
        _ => Err(std::io::Error::other("IPv4 アドレスがありません")),
    }
}

/// Third octets of `192.168.x.0/24` or `172.x.0.0/16` (`x` from `local`), nearest to `local` first.
fn blocks(local: Ipv4Addr) -> Vec<u8> {
    match local.octets() {
        [192, 168, third, _] => vec![third],
        [172, _, third, _] => {
            let mut all: Vec<u8> = (0..=255).collect();
            all.sort_by_key(|block| block.abs_diff(third));
            all
        }
        _ => Vec::new(),
    }
}

/// Hosts in `blocks`, without the network and broadcast addresses, and without `local`.
fn targets(local: Ipv4Addr) -> impl Iterator<Item = Ipv4Addr> {
    let blocks = blocks(local);
    let host_mask: u32 = if blocks.len() == 1 { 0xff } else { 0xffff };
    let [a, b, _, _] = local.octets();
    blocks
        .into_iter()
        .flat_map(move |third| (0..=255).map(move |last| Ipv4Addr::new(a, b, third, last)))
        .filter(move |addr| {
            let host = u32::from(*addr) & host_mask;
            *addr != local && host != 0 && host != host_mask
        })
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
    fn targets_follow_the_local_prefix() {
        let local = Ipv4Addr::new(192, 168, 1, 10);
        let all: Vec<_> = targets(local).collect();
        assert_eq!(all.len(), 253);
        assert_eq!(all[0], Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(*all.last().unwrap(), Ipv4Addr::new(192, 168, 1, 254));
        assert!(!all.contains(&local));

        let local = Ipv4Addr::new(172, 24, 163, 106);
        let all: Vec<_> = targets(local).collect();
        assert_eq!(all.len(), (1 << 16) - 3);
        assert_eq!(all[0], Ipv4Addr::new(172, 24, 163, 0));
        assert_eq!(all[255], Ipv4Addr::new(172, 24, 162, 0));
        assert!(!all.contains(&local));
        assert!(!all.contains(&Ipv4Addr::new(172, 24, 0, 0)));
        assert!(!all.contains(&Ipv4Addr::new(172, 24, 255, 255)));
        assert!(all.contains(&Ipv4Addr::new(172, 24, 255, 254)));
        assert!(!all.contains(&Ipv4Addr::new(172, 0, 0, 1)));

        assert_eq!(targets(Ipv4Addr::new(10, 1, 2, 3)).count(), 0);
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
