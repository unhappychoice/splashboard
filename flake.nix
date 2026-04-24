{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      pname = cargoToml.package.name;
      version = cargoToml.package.version;
      supportedSystems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = f: nixpkgs.lib.genAttrs supportedSystems (system:
        f {
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
        }
      );
    in
    {
      description = cargoToml.package.description;
      packages = forAllSystems ({ pkgs }: {
        default = pkgs.rustPlatform.buildRustPackage rec {
          pname = "splashboard";
          version = "0.2.0";
          src = pkgs.fetchFromGitHub {
            owner = "unhappychoice";
            repo = "splashboard";
            rev = "v${version}";
            hash = "sha256-CFJuBt8ef+MI/dRWLUJ2OGopEt7aBhU7vfVkmNMU+fA=";
          };
          cargoHash = "sha256-hm+FZoFkocMejoi36Xixb/YuGB5B+nxgLlPkKQGGP4Q=";
          nativeBuildInputs = [ pkgs.pkg-config pkgs.git ];
          doCheck = false;
        };

        unstable = pkgs.rustPlatform.buildRustPackage rec {
          inherit pname version;
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [ pkgs.pkg-config pkgs.git ];
          doCheck = false;
        };
      });

      devShells = forAllSystems ({ pkgs }: {
        default = pkgs.mkShell {
          buildInputs = [
            pkgs.rust-bin.stable.latest.default
            pkgs.pkg-config
            pkgs.git
          ];
        };
      });

      defaultPackage = forAllSystems ({ pkgs }: self.packages.${pkgs.system}.default);
      defaultDevShell = forAllSystems ({ pkgs }: self.devShells.${pkgs.system}.default);

      apps = forAllSystems ({ pkgs }: {
        default = {
          type = "app";
          program = "${self.packages.${pkgs.system}.default}/bin/splashboard";
        };
        unstable = {
          type = "app";
          program = "${self.packages.${pkgs.system}.unstable}/bin/splashboard";
        };
      });
    };
}
