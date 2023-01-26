# Data sources

To hook up routers to your looking glass, you will have to configure either a BMP (BGP Monitoring Protocol) or BGP session between your router and the looking glass.

When the router supports it, BMP is always the preferred option.

For both the BGP and BMP collectors, multiple instances can be created (listening on different ports, etc.) and per-peer configuration can be provided based on the client IP.

```yml

collectors:

  # BMP collector that listens on port 11019 and accepts all incoming connections
  - collector_type: Bmp
    bind: "[::]:11019"
    default_peer_config: {}

  # BMP collector that listens on the privileged port 11020 and accepts incoming connections only from select client IPs
  - collector_type: Bmp
    bind: "[::]:11020"
    peers:
      "192.0.2.1": {}
      "192.0.2.2":
        name_override: router02.example.org

  # BGP collector that listens on port 1179 and accept all  incoming connections
  - collector_type: Bgp
    bind: "[::]:1179"
    default_peer_config:
      asn: 64496
      router_id: 192.0.2.100

  # BGP collector that listens on the privileged port 179 and accepts incoming connections only from select client IPs
  - collector_type: Bgp
    bind: "[::]:179"
    peers:
      "192.0.2.1":
        asn: 64496
	router_id: 192.0.2.100
      "192.0.2.2":
        asn: 64496
	router_id: 192.0.2.100
        name_override: router02.example.org
```

Valid options for BMP peer config:

- `name_override` (optional): Use this string instead of the `sys_name` advertised in the BMP initiation message

Valid options for BGP peer config:

- `asn` (required): AS Number advertised to peer
- `router_id` (required): Router ID advertised to peer
- `name_override` (optional): Use this string instead of the hostname advertised in the [BGP hostname capability](https://www.ietf.org/archive/id/draft-walton-bgp-hostname-capability-02.txt)
