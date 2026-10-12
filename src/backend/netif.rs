//! IPv4 addresses of the local network interfaces.

use std::net::Ipv4Addr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    pub name: String,
    pub addr: Ipv4Addr,
    pub prefix: u8,
}

fn prefix_of(mask: Ipv4Addr) -> u8 {
    u32::from(mask).leading_ones() as u8
}

/// Interfaces that are up, without loopback.
#[cfg(windows)]
pub fn interfaces() -> Vec<Interface> {
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    const MIB_IPADDR_DISCONNECTED: u16 = 0x0008;
    const MIB_IPADDR_DELETED: u16 = 0x0040;
    /// `MIB_IPADDRROW`: addr, index, mask, broadcast, reassembly size, then `unused1` and
    /// `wType` as the low and high halves of the sixth word.
    const ROW_WORDS: usize = 6;

    let mut size = 0u32;
    let code = unsafe { GetIpAddrTable(std::ptr::null_mut(), &mut size, 0) };
    if code != ERROR_INSUFFICIENT_BUFFER || size == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u32; (size as usize).div_ceil(4)];
    if unsafe { GetIpAddrTable(buf.as_mut_ptr(), &mut size, 0) } != 0 {
        return Vec::new();
    }
    let count = buf[0] as usize;
    buf[1..]
        .as_chunks::<ROW_WORDS>()
        .0
        .iter()
        .take(count)
        .filter_map(|row| {
            let addr = Ipv4Addr::from(row[0].to_ne_bytes());
            let flags = (row[5] >> 16) as u16;
            if addr.is_loopback()
                || addr.is_unspecified()
                || flags & (MIB_IPADDR_DISCONNECTED | MIB_IPADDR_DELETED) != 0
            {
                return None;
            }
            Some(Interface {
                name: alias_of(row[1]).unwrap_or_else(|| format!("#{}", row[1])),
                addr,
                prefix: prefix_of(Ipv4Addr::from(row[2].to_ne_bytes())),
            })
        })
        .collect()
}

#[cfg(windows)]
fn alias_of(index: u32) -> Option<String> {
    let mut luid = 0u64;
    if unsafe { ConvertInterfaceIndexToLuid(index, &mut luid) } != 0 {
        return None;
    }
    let mut name = [0u16; 257];
    if unsafe { ConvertInterfaceLuidToAlias(&luid, name.as_mut_ptr(), name.len()) } != 0 {
        return None;
    }
    let len = name.iter().position(|&c| c == 0)?;
    (len > 0).then(|| String::from_utf16_lossy(&name[..len]))
}

#[cfg(windows)]
#[link(name = "iphlpapi")]
unsafe extern "system" {
    fn GetIpAddrTable(table: *mut u32, size: *mut u32, order: i32) -> u32;
    fn ConvertInterfaceIndexToLuid(index: u32, luid: *mut u64) -> u32;
    fn ConvertInterfaceLuidToAlias(luid: *const u64, alias: *mut u16, len: usize) -> u32;
}

/// Interfaces that are up, without loopback.
#[cfg(target_os = "linux")]
pub fn interfaces() -> Vec<Interface> {
    const AF_INET: u16 = 2;
    const IFF_UP: u32 = 0x1;
    const IFF_LOOPBACK: u32 = 0x8;

    let mut head: *mut IfAddrs = std::ptr::null_mut();
    if unsafe { getifaddrs(&mut head) } != 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cursor = head;
    while !cursor.is_null() {
        let entry = unsafe { &*cursor };
        cursor = entry.next;
        if entry.flags & IFF_UP == 0
            || entry.flags & IFF_LOOPBACK != 0
            || entry.addr.is_null()
            || entry.netmask.is_null()
        {
            continue;
        }
        let (addr, mask) = unsafe { (&*entry.addr, &*entry.netmask) };
        if addr.family != AF_INET {
            continue;
        }
        let name = unsafe { std::ffi::CStr::from_ptr(entry.name) }
            .to_string_lossy()
            .into_owned();
        out.push(Interface {
            name,
            addr: Ipv4Addr::from(addr.addr),
            prefix: prefix_of(Ipv4Addr::from(mask.addr)),
        });
    }
    unsafe { freeifaddrs(head) };
    out
}

#[cfg(target_os = "linux")]
#[repr(C)]
struct SockaddrIn {
    family: u16,
    port: u16,
    addr: [u8; 4],
}

#[cfg(target_os = "linux")]
#[repr(C)]
struct IfAddrs {
    next: *mut IfAddrs,
    name: *const std::ffi::c_char,
    flags: u32,
    addr: *const SockaddrIn,
    netmask: *const SockaddrIn,
    broadcast: *const std::ffi::c_void,
    data: *const std::ffi::c_void,
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn getifaddrs(ifap: *mut *mut IfAddrs) -> i32;
    fn freeifaddrs(ifa: *mut IfAddrs);
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn interfaces() -> Vec<Interface> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_from_mask() {
        assert_eq!(prefix_of(Ipv4Addr::new(255, 255, 255, 0)), 24);
        assert_eq!(prefix_of(Ipv4Addr::new(255, 255, 240, 0)), 20);
        assert_eq!(prefix_of(Ipv4Addr::new(255, 255, 255, 255)), 32);
    }
}
