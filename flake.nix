{

  description = "fernglas";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.flake-utils.url = "github:numtide/flake-utils";

  outputs = { self, nixpkgs, flake-utils }: {
    overlays.default = final: prev: {
      fernglas = final.callPackage ({ rustPlatform }:

        rustPlatform.buildRustPackage {
          pname = "fernglas";
          version =
            self.shortRev or "dirty-${toString self.lastModifiedDate}";
          src = self;
          cargoBuildFlags = [ "--all-features" ];
          cargoLock = {
            lockFile = ./Cargo.lock;
            allowBuiltinFetchGit = true;
          };
        }
      ) { };

    };
  } // flake-utils.lib.eachDefaultSystem (system: let
    pkgs = import nixpkgs {
      inherit system;
      overlays = [ self.overlays.default ];
    };
  in rec {
    packages = {
      inherit (pkgs) fernglas;
      default = packages.fernglas;
    };
  });
}
