# Backend

Download a statically linked binary from CI artifacts (link TBD) and place it at `/usr/local/bin/fernglas`. Make it executable.

File: /etc/fernglas/config.yml
```yaml
api:
  bind: "[::1]:3000"
collectors:
  - collector_type: Bmp
    bind: "[::]:11019"
    peers:
      "192.0.2.1": {}
```

systemd service with hardening options:

File: /etc/systemd/system/fernglas.service
```ini
[Service]
ExecStart=/usr/local/bin/fernglas /etc/fernglas/config.yml
Environment=RUST_LOG=warn,fernglas=info
Restart=always
RestartSec=10
DynamicUser=true
DevicePolicy=closed
MemoryDenyWriteExecute=true
NoNewPrivileges=true
PrivateDevices=true
PrivateTmp=true
ProtectControlGroups=true
ProtectHome=true
ProtectSystem=strict
```

Optionally, add `AmbientCapabilities=CAP_NET_BIND_SERVICE` if your configuration requires binding to privileged ports.

Enable and start the service:

`systemctl enable --now fernglas.service`

Don't forget to open the appropriate firewall ports if necessary!
