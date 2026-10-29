{
  description = "LG Buddy package and declarative NixOS/Home Manager integration";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
  inputs.home-manager = {
    url = "github:nix-community/home-manager/release-26.05";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      home-manager,
    }:
    let
      forAllSystems = nixpkgs.lib.genAttrs [
        "x86_64-linux"
        "aarch64-linux"
      ];
      channels = [
        "dev"
        "stable"
        "prerelease"
      ];
      version = (builtins.fromTOML (builtins.readFile ./crates/lg-buddy/Cargo.toml)).package.version;
    in
    {
      nixosModules =
        nixpkgs.lib.genAttrs channels (
          channel:
          import ./nix/modules/nixos.nix {
            source = self;
            inherit channel;
          }
        )
        // {
          lg-buddy = import ./nix/modules/nixos.nix { source = self; };
          default = self.nixosModules.lg-buddy;
        };
      homeManagerModules =
        nixpkgs.lib.genAttrs channels (
          channel:
          import ./nix/modules/home-manager.nix {
            source = self;
            inherit channel;
          }
        )
        // {
          lg-buddy = import ./nix/modules/home-manager.nix { source = self; };
          default = self.homeManagerModules.lg-buddy;
        };

      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          packageFor =
            channel:
            pkgs.callPackage ./nix/package.nix {
              inherit channel;
              src = self;
              buildCommit = self.rev or self.dirtyRev or "unknown";
            };
        in
        {
          lg-buddy = packageFor "release";
          lg-buddy-dev = packageFor "dev";
          lg-buddy-prerelease = packageFor "prerelease";
          lg-buddy-kwin = pkgs.callPackage ./nix/kwin.nix { package = self.packages.${system}.lg-buddy; };
          default = self.packages.${system}.lg-buddy;
        }
        // nixpkgs.lib.optionalAttrs (!(nixpkgs.lib.hasInfix "-" version)) {
          lg-buddy-stable = packageFor "stable";
        }
      );

      checks = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          package = pkgs.callPackage ./nix/check-package.nix {
            package = self.packages.${system}.lg-buddy;
          };
          module-evaluation = import ./nix/tests/evaluation.nix {
            inherit
              self
              nixpkgs
              home-manager
              pkgs
              ;
          };
          launcher = import ./nix/tests/launcher.nix {
            inherit self home-manager pkgs;
          };
          kwin = pkgs.callPackage ./nix/check-kwin.nix {
            package = self.packages.${system}.lg-buddy;
            bridge = self.packages.${system}.lg-buddy-kwin;
          };
        }
        // nixpkgs.lib.optionalAttrs (system == "x86_64-linux") {
          modules = import ./nix/tests/modules.nix {
            inherit self home-manager pkgs;
            package = self.packages.${system}.lg-buddy;
          };
        }
      );
    };
}
