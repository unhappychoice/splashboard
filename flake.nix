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
          version = "1.2.2";
          src = pkgs.fetchFromGitHub {
            owner = "unhappychoice";
            repo = "splashboard";
            rev = "v${version}";
            hash = "sha256-AxajajFRzLl93CQS6RZ7q7prtLT5Qd4PRPpQp622r+M=";
          };
          cargoHash = "sha256-jH/tCsHZsqPcvpCl9Pwor4G5ZQwEXseeebItiifr4d8=";
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
