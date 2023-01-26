# NixOS specialArgs pattern

### Borrowed from [here](https://pad.yuka.dev/s/DpS0wJ4R6#)

problem: you want to get the home-manager nixos module from the home-manager flake into a nixos config:

the home-manager documentation says this: https://nix-community.github.io/home-manager/index.html#sec-flakes-nixos-module

```nix
{
  description = "NixOS configuration";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    home-manager.url = "github:nix-community/home-manager";
    home-manager.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = inputs@{ nixpkgs, home-manager, ... }: {
    nixosConfigurations = {
      hostname = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          ./configuration.nix
          home-manager.nixosModules.home-manager
          {
            home-manager.useGlobalPkgs = true;
            home-manager.useUserPackages = true;
            home-manager.users.jdoe = import ./home.nix;

            # Optionally, use home-manager.extraSpecialArgs to pass
            # arguments to home.nix
          }
        ];
      };
    };
  };
}
```

I find this ugly, because it forces to have the home manager include (`modules = [ ... home-manager.nixosModules.home-manager ... 
 ];`) in the flake.nix, because only there the inputs or home-manager attrset is in scope

In "old" configurations, with niv or plain fetchTarball, you would have done this in the configuration.nix of the respective host(, or a common.nix, if it should be included on all hosts)
`imports = [ <home-manager/nixos> ];`
actually _anywhere_ in _any_ nixos config file

The solution is the following:

flake.nix
```nix

{
  description = "NixOS configuration";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    home-manager.url = "github:nix-community/home-manager";
    home-manager.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = inputs@{ nixpkgs, ... }: {
    nixosConfigurations = {
      hostname = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        specialArgs = { inherit inputs; };
        modules = [
          ./configuration.nix
        ];
      };
    };
  };
}
```

configuration.nix
```nix
{ pkgs, lib, config, inputs, ... }:

{
  networking.hostName = "foo";
  [...]

  imports = [
    inputs.home-manager.nixosModules.home-manager
  ];
  home-manager.useGlobalPkgs = true;
  home-manager.useUserPackages = true;
  home-manager.users.jdoe = import ./home.nix;
}
```

specialArgs means, the `inputs` attrset is now available in the module args in every nixos module. just add it to the function parameters anywhere you need it, like you do it with `pkgs`, `lib` and `config`.
specialArgs also means that in contrast to `_module.args` this parameter to the module system is fixed, and can not be changed by nixos modules themselves. this prevents infinite recursions when using stuff from the inputs attrset in nixos module imports (which is exactly what we want to do).

and like this using flake inputs in nixos configs becomes much easier and more natural. in many cases you can rewrite "old" configs 1:1 to this new pattern without moving the includes to flake.nix
