{
  description = "Per-pin 12V-2x6 watchdog for ASUS ROG Astral and Matrix GPUs and the WireView Pro II";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (pkgs: rec {
        gpu-warden = pkgs.callPackage ./packaging/nix/package.nix { };
        default = gpu-warden;
      });

      overlays.default = final: _: {
        gpu-warden = final.callPackage ./packaging/nix/package.nix { };
      };

      nixosModules.default = import ./packaging/nix/module.nix self;

      checks = forAllSystems (pkgs: {
        package = self.packages.${pkgs.stdenv.hostPlatform.system}.default;

        module =
          (nixpkgs.lib.nixosSystem {
            inherit (pkgs.stdenv.hostPlatform) system;
            modules = [
              self.nixosModules.default
              {
                services.gpu-warden = {
                  enable = true;
                  settings.power_cap.percent = 70;
                };
                boot.loader.grub.enable = false;
                fileSystems."/" = {
                  device = "none";
                  fsType = "tmpfs";
                };
                system.stateVersion = "25.11";
              }
            ];
          }).config.system.build.etc;
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt);
    };
}
