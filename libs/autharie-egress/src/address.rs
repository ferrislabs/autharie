use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(address: Ipv4Addr) -> bool {
    let [a, b, c, _] = address.octets();

    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_multicast()
        || address.is_broadcast()
        || a == 0
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 240)
}

fn is_public_v6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_v4(mapped);
    }

    let segments = address.segments();
    let embedded = |high: u16, low: u16| {
        Ipv4Addr::new(
            (high >> 8) as u8,
            (high & 0xff) as u8,
            (low >> 8) as u8,
            (low & 0xff) as u8,
        )
    };

    if segments[..6] == [0, 0, 0, 0, 0, 0] {
        return false;
    }
    if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return is_public_v4(embedded(segments[6], segments[7]));
    }
    if segments[0] == 0x2002 {
        return is_public_v4(embedded(segments[1], segments[2]));
    }

    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] & 0xffc0) == 0xfec0
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || (segments[0] == 0x0100 && segments[1..4] == [0, 0, 0]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("an address")
    }

    #[test]
    fn private_loopback_and_link_local_ranges_are_refused() {
        for text in [
            "10.0.0.5",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "127.0.0.1",
            "127.8.8.8",
            "169.254.169.254",
            "169.254.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
        ] {
            assert!(!is_public(ip(text)), "{text}");
        }
    }

    #[test]
    fn carrier_grade_nat_reserved_and_documentation_ranges_are_refused() {
        for text in [
            "100.64.0.1",
            "100.100.100.200",
            "100.127.255.255",
            "192.0.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.7",
            "203.0.113.9",
            "240.0.0.1",
        ] {
            assert!(!is_public(ip(text)), "{text}");
        }
    }

    #[test]
    fn ordinary_public_addresses_are_accepted() {
        for text in [
            "1.1.1.1",
            "8.8.8.8",
            "93.184.216.34",
            "100.63.255.255",
            "100.128.0.1",
            "172.15.0.1",
            "172.32.0.1",
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
        ] {
            assert!(is_public(ip(text)), "{text}");
        }
    }

    #[test]
    fn ipv6_loopback_link_local_unique_local_and_metadata_are_refused() {
        for text in [
            "::",
            "::1",
            "fe80::1",
            "febf::1",
            "fec0::1",
            "fc00::1",
            "fd00:ec2::254",
            "ff02::1",
            "2001:db8::1",
            "100::1",
        ] {
            assert!(!is_public(ip(text)), "{text}");
        }
    }

    #[test]
    fn an_ipv4_address_wearing_an_ipv6_costume_is_judged_as_ipv4() {
        for text in [
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:10.0.0.1",
            "::127.0.0.1",
            "64:ff9b::7f00:1",
            "64:ff9b::a9fe:a9fe",
            "2002:7f00:1::1",
            "2002:0a00:0001::1",
        ] {
            assert!(!is_public(ip(text)), "{text}");
        }
        assert!(is_public(ip("::ffff:8.8.8.8")));
        assert!(is_public(ip("64:ff9b::808:808")));
    }
}
