# Configuration

The example configuration uses port `3000` and `11019` to expose the API and collect the BMP data stream. You can change those ports, if needed,
but you need to expose 11019 to an IP address reachable from your router, probably bind this port to `[::]:11019` and check for outside reachability.
Note: You also need to specify the IP addresses of possible peers in the config file to ensure no unauthorized person is steaming a BMP stream to your machine.

To hook up routers to your looking glass, you will have to configure either a BMP (BGP Monitoring Protocol) or BGP session between your router and the looking glass.

For both the BGP and BMP collectors, multiple instances can be created (listening on different ports, etc.) and per-peer configuration can be provided based on the client IP.

If multiple collectors collect data with the same hostname (as reported by BMP or BGP peer, or set in `name_override`), the data will be combined in the frontend. This can be used to build complex views of the Pre/Post Policy Adj-In and LocRib tables using multiple BGP sessions.  
If using BMP, everything should 'just work'.

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
