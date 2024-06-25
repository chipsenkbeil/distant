{
  description = "Library and tooling that supports remote filesystem and process operations";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }: {
    packages = builtins.listToAttrs (map (systemName: {
      name = systemName;
      value = { default =
        with import nixpkgs { system = systemName; };
        pkgs.rustPlatform.buildRustPackage rec {
          name = "distant";

          # Update this whenever you release a new version. Make sure to omit the 'v'.
          version = "0.20.0-unstable";
          src = self;

          # Update this whenever you update Cargo.lock
          cargoHash = "sha256-mPcrfBFgvbPi6O7i9FCtN3iaaEOHIcDFHCOpV1NxKMY=";

          # Build time
          nativeBuildInputs = with pkgs; [ perl ];

          meta = {
            pname = "distant";
            description = "Library and tooling that supports remote filesystem and process operations.";
            longDescription = ''
              Libraries and tooling for working remotely.
              Deploy distant on your server and connect today to begin remote work!
            '';
            homepage = "https://distant.dev";
            license = with lib.licenses; [ mit asl20 ];
          };
        };
      };
    })
    [ "x86_64-linux" "armv7l-linux"
      "aarch64-darwin" "x86_64-darwin" ]);
  };
}
