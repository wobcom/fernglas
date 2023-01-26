# bird2 using BGP

```
protocol bgp bgp_lg from bgp_all {
  local as 64496;
  source address 2001:db8::1;
  neighbor 2001:db8::100 port 1179 as 64496;
  multihop 64;
  rr client;
  advertise hostname on;

  ipv6 {
    add paths tx;
    import filter { reject; };
    export filter { accept; };
    next hop keep;
  };
  ipv4 {
    add paths tx;
    import filter { reject; };
    export filter { accept; };
    next hop keep;
  };
}
```
