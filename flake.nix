{
  description = "Operate on a remote computer through file and process manipulation";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, crane, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        inherit (pkgs) lib;
        craneLib = crane.mkLib pkgs;

        # Include standard Cargo sources plus:
        # - patches/ directory ([patch.crates-io] path dependency)
        # - README.md files (referenced by include_str! in lib crates)
        # - .toml files in src/ (embedded default config)
        src = lib.cleanSourceWith {
          src = ./.;
          filter = path: type:
            (craneLib.filterCargoSources path type)
            || (builtins.match ".*patches/.*" path != null)
            || (builtins.match ".*README\\.md$" path != null)
            || (builtins.match ".*/src/.*\\.toml$" path != null);
        };

        commonArgs = {
          inherit src;
          strictDeps = true;

          buildInputs = lib.optionals pkgs.stdenv.isDarwin (with pkgs; [
            libiconv
            apple-sdk_15
          ]);

          # Crane's mkDummySrc replaces all .rs files with stubs during the
          # deps-only build.  The [patch.crates-io] path dependency under
          # patches/nix-0.29.0 must keep its real source so dependent crates
          # (russh-cryptovec) can resolve its public API.
          dummySrc = craneLib.mkDummySrc {
            inherit src;
            extraDummyScript = ''
              rm -rf $out/patches
              cp -r --no-preserve=mode ${src}/patches $out/patches
            '';
          };
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        distant = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          doCheck = false; # Tests require network/Docker
        });
      in
      {
        packages = {
          default = distant;
          distant = distant;
        };

        devShells.default = craneLib.devShell {
          packages = lib.optionals pkgs.stdenv.isDarwin (with pkgs; [
              libiconv
              apple-sdk_15
            ]);
        };
      }
    );
}
