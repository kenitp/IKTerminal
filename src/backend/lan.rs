//! LAN discovery: hosts in fixed IPv4 ranges that accept TCP on a port.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::task::JoinSet;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(800);
const PARALLEL: usize = 64;

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
    Done,
}

/// Scan results collected for the sidebar.
#[derive(Default)]
pub struct Scanner {
    rx: Option<Receiver<Event>>,
    pub devices: Vec<Device>,
    pub message: Option<String>,
}

impl Scanner {
    pub fn scanning(&self) -> bool {
        self.rx.is_some()
    }

    pub fn start(&mut self, port: u16, wake: impl Fn() + Send + Sync + 'static) {
        self.devices.clear();
        self.message = None;
        match local_ipv4() {
            Ok(local) => self.rx = Some(spawn(local, port, Arc::new(wake))),
            Err(error) => self.message = Some(format!("スキャンできません: {error}")),
        }
    }

    /// Takes finished results. Devices for which `keep` is false are skipped.
    pub fn poll(&mut self, keep: impl Fn(&Device) -> bool) {
        let Some(rx) = &self.rx else { return };
        while let Ok(event) = rx.try_recv() {
            match event {
                Event::Found(device) => {
                    if keep(&device) {
                        let at = self.devices.partition_point(|d| d.addr < device.addr);
                        self.devices.insert(at, device);
                    }
                }
                Event::Done => {
                    self.rx = None;
                    if self.devices.is_empty() {
                        self.message = Some("未登録の機器は見つかりません".to_owned());
                    }
                    return;
                }
            }
        }
    }

    pub fn retain(&mut self, keep: impl Fn(&Device) -> bool) {
        self.devices.retain(keep);
    }
}

fn spawn(local: Ipv4Addr, port: u16, wake: Arc<dyn Fn() + Send + Sync>) -> Receiver<Event> {
    let (tx, rx) = mpsc::channel();
    super::runtime().spawn(async move {
        let mut addrs = targets(local);
        let mut tasks = JoinSet::new();
        loop {
            while tasks.len() < PARALLEL
                && let Some(addr) = addrs.next()
            {
                let tx = tx.clone();
                let wake = wake.clone();
                tasks.spawn(async move {
                    let open = tokio::time::timeout(
                        CONNECT_TIMEOUT,
                        TcpStream::connect(SocketAddr::from((addr, port))),
                    )
                    .await
                    .is_ok_and(|r| r.is_ok());
                    if open {
                        found(&tx, addr, port).await;
                        wake();
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

async fn found(tx: &Sender<Event>, addr: Ipv4Addr, port: u16) {
    let name = tokio::task::spawn_blocking(move || reverse_name(addr))
        .await
        .ok()
        .flatten();
    let _ = tx.send(Event::Found(Device { addr, port, name }));
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

const RANGES: [(Ipv4Addr, u32); 2] = [
    (Ipv4Addr::new(192, 168, 0, 0), 24),
    (Ipv4Addr::new(172, 0, 0, 0), 16),
];

/// Hosts in `RANGES`, without each prefix's network and broadcast address, and without `local`.
fn targets(local: Ipv4Addr) -> impl Iterator<Item = Ipv4Addr> {
    RANGES.into_iter().flat_map(move |(base, prefix)| {
        let start = u32::from(base);
        let count = 1u32 << (32 - prefix);
        (1..count - 1)
            .map(move |offset| Ipv4Addr::from(start + offset))
            .filter(move |addr| *addr != local)
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
    fn targets_are_the_two_ranges_without_self() {
        let local = Ipv4Addr::new(192, 168, 0, 10);
        let all: Vec<_> = targets(local).collect();
        assert_eq!(all[0], Ipv4Addr::new(192, 168, 0, 1));
        assert_eq!(all.len(), 253 + (1 << 16) - 2);
        assert!(!all.contains(&local));
        assert!(!all.contains(&Ipv4Addr::new(192, 168, 0, 0)));
        assert!(!all.contains(&Ipv4Addr::new(192, 168, 1, 1)));
        assert!(all.contains(&Ipv4Addr::new(172, 0, 0, 1)));
        assert_eq!(*all.last().unwrap(), Ipv4Addr::new(172, 0, 255, 254));
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
