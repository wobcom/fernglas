{

  description = "fernglas";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/master";
  inputs.flake-utils.url = "github:numtide/flake-utils";

  outputs = { self, nixpkgs, flake-utils }: {
    overlays.default = final: prev: {
      fernglas = final.callPackage (
        { lib, stdenv, rustPlatform }:

        rustPlatform.buildRustPackage {
          pname = "fernglas";
          version =
            self.shortRev or "dirty-${toString self.lastModifiedDate}";
          src = lib.cleanSourceWith {
            filter = lib.cleanSourceFilter;
            src = lib.cleanSourceWith {
              filter =
                name: type: !(lib.hasInfix "/frontend" name)
                && !(lib.hasInfix "/manual" name);
              src = self;
            };
          };

          cargoBuildFlags = lib.optionals (stdenv.hostPlatform.isMusl && stdenv.hostPlatform.isStatic) [ "--features" "mimalloc" ];
          cargoLock = {
            lockFile = ./Cargo.lock;
            allowBuiltinFetchGit = true;
          };
        }
      ) { };

      fernglas-frontend = final.callPackage (
        { lib, stdenv, yarn2nix-moretea, yarn, nodejs-slim }:

        stdenv.mkDerivation {
          pname = "fernglas-frontend";
          version =
            self.shortRev or "dirty-${toString self.lastModifiedDate}";

          src = lib.cleanSourceWith {
            filter = lib.cleanSourceFilter;
            src = lib.cleanSourceWith {
              filter =
                name: type: !(lib.hasInfix "node_modules" name)
                && !(lib.hasInfix "dist" name);
              src = ./frontend;
            };
          };

          offlineCache = let
            yarnLock = ./frontend/yarn.lock;
            yarnNix = yarn2nix-moretea.mkYarnNix { inherit yarnLock; };
          in
            yarn2nix-moretea.importOfflineCache yarnNix;

          nativeBuildInputs = [ yarn nodejs-slim yarn2nix-moretea.fixup_yarn_lock ];

          configurePhase = ''
            runHook preConfigure

            export HOME=$NIX_BUILD_TOP/fake_home
            yarn config --offline set yarn-offline-mirror $offlineCache
            fixup_yarn_lock yarn.lock
            yarn install --offline --frozen-lockfile --ignore-scripts --no-progress --non-interactive
            patchShebangs node_modules/

            runHook postConfigure
          '';

          buildPhase = ''
            runHook preBuild
            node_modules/.bin/webpack
            runHook postBuild
          '';

          installPhase = ''
            runHook preInstall
            mv dist $out
            runHook postInstall
          '';
        }

      ) { };

      fernglas-manual = final.callPackage (
        { lib, stdenv, mdbook }:

        stdenv.mkDerivation {
          name = "fernglas-manual";
          src = lib.cleanSource ./manual;
          nativeBuildInputs = [ mdbook ];
          buildPhase = ''
            mdbook build -d ./build
          '';
          installPhase = ''
            cp -r ./build $out
          '';
        }
      ) { };
    };

    nixConfig = {
      extra-substituters = [ "wobcom-public.cachix.org" ];
      extra-trusted-public-keys = [ "wobcom-public.cachix.org-1:bEm3vZ3mRNLDLMyFwPqgArvOR6vGpVtxCYLyp+r0An8=" ];
    };

  } // flake-utils.lib.eachDefaultSystem (system: let
    pkgs = import nixpkgs {
      inherit system;
      overlays = [ self.overlays.default ];
    };
  in rec {
    packages = {
      inherit (pkgs) fernglas fernglas-frontend;
      default = packages.fernglas;
    };
    legacyPackages = pkgs;
  });
}
