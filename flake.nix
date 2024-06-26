{
  description = "Library and tooling that supports remote filesystem and process operations";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.flake-utils.url = "github:numtide/flake-utils";

  outputs = { self, nixpkgs, flake-utils }: 
    flake-utils.lib.eachDefaultSystem (system:
      with import nixpkgs { system = system; }; {
        packages.default =
          pkgs.rustPlatform.buildRustPackage rec {
            name = "distant";

            src = self;

            # Update this whenever you update Cargo.lock
            cargoHash = "sha256-mPcrfBFgvbPi6O7i9FCtN3iaaEOHIcDFHCOpV1NxKMY=";

            # Build time
            nativeBuildInputs = with pkgs; [ perl ];

            meta = {
              pname = "distant";
              license = with lib.licenses; [ mit asl20 ];
            };
          };
      }
    );
}
