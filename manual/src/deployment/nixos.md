# Deploying fernglas using NixOS

Requirements:

- Use NixOS with Nix flakes for your deploment
- Be aware of [the specialArgs pattern in NixOS with flakes](/appendix/nixos-specialArgs-pattern.md) to get the `inputs` module arg


Optional: Set up the binary cache to use prebuilt binaries from our CI

```
$ nix run nixpkgs#cachix use wobcom-public
```

Add fernglas to your flake inputs:

```nix
inputs.fernglas = {
  type = "github";
  owner = "wobcom";
  repo = "fernglas";
};
```

Import the fernglas NixOS module and declare your configuration.

```nix
{ inputs, ... }:

let
  bmpPort = 11019;
in {
  imports = [
    inputs.fernglas.nixosModules.default
  ];

  services.fernglas = {
    enable = true;
    settings = {
      api.bind = "[::1]:3000";
      collectors = {
        my_bmp_collector = {
          collector_type = "Bmp";
          bind = "[::]:${toString bmpPort}";
          peers = {
            "192.0.2.1" = {};
          };
        };
      };
    };
  };

  networking.firewall.allowedTCPPorts = [ bmpPort ];
}
```

Configure a reverse proxy for the API and a webserver to serve the frontend.

```nix
{ config, inputs, ... }:

{
  services.nginx = {
    enable = true;
    recommendedProxySettings = true;
    virtualHosts."lg.example.org" = {
      enableACME = true;
      forceSSL = true;
      locations."/".root = inputs.fernglas.packages.${config.nixpkgs.hostPlatform.system}.fernglas-frontend;
      locations."/api/".proxyPass = "http://${config.services.fernglas.settings.api.bind}";
    };
  };

  networking.firewall.allowedTCPPorts = [ 80 443 ];
}
```
