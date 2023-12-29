# Junos using BMP

```
routing-options {
    bmp {
        station looking-glass {
            station-address 2001:db8::100;
            station-port 11019;
            local-address 2001:db8::1;
            connection-mode active;
            route-monitoring {
                pre-policy { exclude-non-feasible; }
                post-policy { exclude-non-eligible; }
                loc-rib;
            }
        }
    }
}
```

Be aware that in some older Junos versions the BMP implementation is buggy and causes memory leaks in the routing process.

| Tested Junos Release | Known Issues |
| ----------- | ----------- |
| 20.2R3-S3.6 | ❌ [PR1526061](https://prsearch.juniper.net/PR1526061) BGP Monitoring Protocols may not releases IO buffer correctly |
| 21.4R3-S2.3 | ❌ [PR1713444](https://prsearch.juniper.net/PR1713444) The rpd process may crash when BMP socket write fails or blocks |
| 21.4R3-S4.9 | ✅ None|
| 22.2R3.15 | ✅ None |
