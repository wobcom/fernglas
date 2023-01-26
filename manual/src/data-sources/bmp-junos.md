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
